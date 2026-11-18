//! WinPanel - a lightweight Windows server management panel.
//!
//! Starts a local management API plus the embedded web interface, supervises
//! game and application processes, and keeps working for weeks at a time.

#![forbid(unsafe_op_in_unsafe_fn)]

mod api;
mod auth;
mod backups;
mod config;
mod db;
mod error;
mod events;
mod files;
mod install;
mod metrics;
mod models;
mod net;
mod paths;
mod process;
mod runtime;
mod state;
mod templates;
mod updates;
mod util;

use crate::config::FileConfig;
use crate::error::{AppError, Result};
use crate::paths::Layout;
use crate::state::AppState;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const BANNER: &str = r#"
  __      __.__        __________
 /  \    /  \__| ____  \______   \_____    ____   ____   ____
 \   \/\/   /  |/    \  |     ___/\__  \  /    \ /    \ /  _ \
  \        /|  |   |  \ |    |     / __ \|   |  \   |  (  <_> )
   \__/\  / |__|___|  / |____|    (____  /___|  /___|  /\____/
        \/          \/                 \/     \/     \/
"#;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = match Cli::parse(std::env::args().skip(1).collect()) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            eprintln!();
            eprintln!("{}", Cli::usage());
            return std::process::ExitCode::FAILURE;
        }
    };
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("WinPanel could not start: {}", err.message());
            std::process::ExitCode::FAILURE
        }
    }
}

struct Cli {
    listen: Option<String>,
    data_dir: Option<PathBuf>,
    allow_remote: bool,
    no_browser: bool,
}

impl Cli {
    fn parse(args: Vec<String>) -> std::result::Result<Self, String> {
        let mut cli = Cli {
            listen: None,
            data_dir: None,
            allow_remote: false,
            no_browser: false,
        };
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    return Err("WinPanel - lightweight server management for Windows".into())
                }
                "--version" | "-V" => {
                    println!("WinPanel {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--listen" => {
                    cli.listen = Some(
                        iter.next()
                            .ok_or_else(|| "--listen requires an address".to_string())?,
                    );
                }
                "--data-dir" => {
                    cli.data_dir = Some(PathBuf::from(
                        iter.next()
                            .ok_or_else(|| "--data-dir requires a path".to_string())?,
                    ));
                }
                "--allow-remote" => cli.allow_remote = true,
                "--no-browser" => cli.no_browser = true,
                other if other.starts_with("--listen=") => {
                    cli.listen = Some(other["--listen=".len()..].to_string());
                }
                other if other.starts_with("--data-dir=") => {
                    cli.data_dir = Some(PathBuf::from(&other["--data-dir=".len()..]));
                }
                other => return Err(format!("Unknown option: {other}")),
            }
        }
        if std::env::var("WINPANEL_ALLOW_REMOTE").as_deref() == Ok("1") {
            cli.allow_remote = true;
        }
        if std::env::var("WINPANEL_NO_BROWSER").as_deref() == Ok("1") {
            cli.no_browser = true;
        }
        Ok(cli)
    }

    fn usage() -> String {
        format!(
            "WinPanel {}\n\nUsage: winpanel [options]\n\n\
             Options:\n  \
             --listen <addr>     Bind address, e.g. 127.0.0.1:8787 (default from config.json)\n  \
             --data-dir <path>   Store the database, servers, backups and logs here\n  \
             --allow-remote      Allow binding to a non-loopback address (also config.json)\n  \
             --no-browser        Do not open the management interface automatically\n  \
             -V, --version       Print the version\n  \
             -h, --help          Print this help\n",
            env!("CARGO_PKG_VERSION")
        )
    }
}

async fn run(cli: Cli) -> Result<()> {
    let data_dir = cli.data_dir.clone().unwrap_or_else(paths::default_data_dir);
    let layout = Layout::new(data_dir);
    layout
        .ensure()
        .map_err(|e| AppError::internal(format!("Cannot create the data directory: {e}")))?;
    let config = FileConfig::load_or_create(&layout.config_file())?;
    init_logging(&layout, &config.log_level)?;

    println!("{BANNER}");
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %layout.data_dir.display(),
        "starting WinPanel"
    );

    let listen_value = cli.listen.clone().unwrap_or_else(|| config.listen.clone());
    let address = FileConfig {
        listen: listen_value.clone(),
        ..config.clone()
    }
    .resolve_listen()?;
    let allow_remote = cli.allow_remote || config.allow_remote;
    if !is_loopback(address.ip()) && !allow_remote {
        return Err(AppError::forbidden(format!(
            "Refusing to bind {address}: remote access is disabled.\n\
             Edit {} and set \"allowRemote\": true (or pass --allow-remote) if you really want\n\
             the panel reachable from other machines. Exposing it requires a firewall rule and\n\
             strong passwords.",
            layout.config_file().display()
        )));
    }

    let db = Arc::new(db::Db::open(&layout.db_file())?);
    let state = AppState::build(db, layout, address)?;
    state.supervisor.reconcile();
    state.db.purge_expired_sessions().ok();

    let first_run = state.db.count_users()? == 0;
    let shown_host = if address.ip() == IpAddr::from([0, 0, 0, 0]) || address.ip().is_unspecified()
    {
        format!("127.0.0.1:{}", address.port())
    } else {
        address.to_string()
    };
    let url = format!("http://{shown_host}/");

    let settings = state.settings();
    spawn_background_tasks(&state, &settings);

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|e| AppError::internal(format!("Cannot bind {address}: {e}")))?;
    let actual = listener
        .local_addr()
        .map_err(|e| AppError::internal(format!("Cannot read the listener address: {e}")))?;

    if !is_loopback(actual.ip()) {
        tracing::warn!("the management API is reachable from the network on {actual}");
    }

    println!("  WinPanel is running");
    println!("  Interface : {url}");
    println!("  Data      : {}", state.layout.data_dir.display());
    if first_run {
        println!("  Setup     : create your administrator account in the browser");
    }
    println!();

    if config.open_browser && !cli.no_browser {
        let url_clone = url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            open_browser(&url_clone);
        });
    }

    let app = api::router(state.clone());
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal());

    if let Err(err) = server.await {
        tracing::error!("HTTP server stopped: {err}");
        state.supervisor.shutdown().await;
        return Err(AppError::internal(format!("HTTP server error: {err}")));
    }
    tracing::info!("shutting down; stopping servers");
    state.supervisor.shutdown().await;
    tracing::info!("goodbye");
    Ok(())
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn spawn_background_tasks(state: &Arc<AppState>, settings: &config::Settings) {
    // Real system + per-server process sampling.
    let metrics = state.metrics.clone();
    let supervisor = state.supervisor.clone();
    metrics::spawn_sampler(metrics, supervisor, settings.metrics_interval_ms);

    // Storage usage per server, refreshed slowly to avoid touching disks often.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(90));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let servers = match state.db.list_servers() {
                    Ok(servers) => servers,
                    Err(err) => {
                        tracing::warn!("cannot list servers for storage sampling: {err}");
                        continue;
                    }
                };
                for server in servers {
                    let dir = PathBuf::from(&server.install_dir);
                    let id = server.id.clone();
                    let supervisor = state.supervisor.clone();
                    tokio::task::spawn_blocking(move || {
                        let size = paths::dir_size(&dir);
                        supervisor.set_disk(&id, size);
                    })
                    .await
                    .ok();
                }
            }
        });
    }

    // Scheduled backups.
    {
        let manager = state.backups.clone();
        tokio::spawn(async move { manager.scheduler().await });
    }

    // Housekeeping: session cleanup, activity trimming, log retention.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(15 * 60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = state.db.purge_expired_sessions() {
                    tracing::warn!("session cleanup failed: {err}");
                }
                if let Err(err) = state.db.prune_activity(5000) {
                    tracing::warn!("activity trim failed: {err}");
                }
            }
        });
    }

    // Update checks (cached, never blocks anything).
    if settings.update_check_enabled {
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let mut ticker = tokio::time::interval(Duration::from_secs(6 * 3600));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let info = state.updates.check(true).await;
                if info.available {
                    tracing::info!("WinPanel {} is available on GitHub", info.latest_version);
                }
            }
        });
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        for candidate in browser_candidates() {
            if candidate.is_file() {
                let result = std::process::Command::new(&candidate)
                    .arg(format!("--app={url}"))
                    .arg("--window-size=1480,940")
                    .spawn();
                if result.is_ok() {
                    return;
                }
            }
        }
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(windows)]
fn browser_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let roots = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"];
    for root in roots {
        if let Ok(base) = std::env::var(root) {
            let base = PathBuf::from(base);
            paths.push(
                base.join("Microsoft")
                    .join("Edge")
                    .join("Application")
                    .join("msedge.exe"),
            );
            paths.push(
                base.join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe"),
            );
        }
    }
    paths
}

/// Append-only log writer used by tracing; rotated at startup when large.
struct FileWriter {
    file: Arc<std::sync::Mutex<std::fs::File>>,
}

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileWriter {
    type Writer = FileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        FileWriter {
            file: self.file.clone(),
        }
    }
}

fn init_logging(layout: &Layout, level: &str) -> Result<()> {
    let log_path = layout.log_dir().join("winpanel.log");
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 10 * 1024 * 1024 {
            let rotated = layout.log_dir().join("winpanel.log.1");
            let _ = std::fs::remove_file(&rotated);
            let _ = std::fs::rename(&log_path, &rotated);
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| AppError::internal(format!("Cannot open the log file: {e}")))?;
    let writer = FileWriter {
        file: Arc::new(std::sync::Mutex::new(file)),
    };
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("winpanel={level},warn"));
    let env_filter = tracing_subscriber::EnvFilter::try_new(&filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
    Ok(())
}
