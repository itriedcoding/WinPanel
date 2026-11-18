//! Server process supervisor: spawn, stream console output, stop gracefully,
//! contain the process tree, detect crashes and restart automatically.

pub mod console;
pub mod guard;

use crate::config::Settings;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{emit, Event, EventBus};
use crate::models::*;
use crate::runtime::resolve_executable;
use crate::util::{now_ms, split_args};
use console::{LogBuffer, LogLine};
use guard::{Limits, ProcessGuard};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

/// Uptime after which a crash no longer counts against the restart budget.
const STABLE_UPTIME: Duration = Duration::from_secs(60);

pub struct Supervisor {
    db: Arc<Db>,
    settings: Arc<RwLock<Settings>>,
    events: EventBus,
    handles: Mutex<HashMap<String, Arc<ServerHandle>>>,
    shutting_down: AtomicBool,
}

pub struct ServerHandle {
    pub id: String,
    pub log: Arc<LogBuffer>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    inner: Mutex<RuntimeInner>,
}

struct RuntimeInner {
    status: String,
    pid: Option<u32>,
    started: Option<Instant>,
    started_ms: Option<i64>,
    stop_requested: bool,
    restarts: u32,
    last_exit: Option<i32>,
    guard: Option<Arc<ProcessGuard>>,
    cpu: f32,
    memory: u64,
    disk: u64,
    install_progress: Option<u8>,
    install_step: Option<String>,
}

impl ServerHandle {
    fn new(id: &str, capacity: usize) -> Self {
        Self {
            id: id.to_string(),
            log: Arc::new(LogBuffer::new(capacity)),
            stdin: tokio::sync::Mutex::new(None),
            inner: Mutex::new(RuntimeInner {
                status: STATUS_STOPPED.to_string(),
                pid: None,
                started: None,
                started_ms: None,
                stop_requested: false,
                restarts: 0,
                last_exit: None,
                guard: None,
                cpu: 0.0,
                memory: 0,
                disk: 0,
                install_progress: None,
                install_step: None,
            }),
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, RuntimeInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_active(&self) -> bool {
        matches!(
            self.inner().status.as_str(),
            STATUS_RUNNING | STATUS_STARTING | STATUS_STOPPING
        )
    }
}

impl Supervisor {
    pub fn new(db: Arc<Db>, settings: Arc<RwLock<Settings>>, events: EventBus) -> Arc<Self> {
        Arc::new(Self {
            db,
            settings,
            events,
            handles: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn handle(&self, id: &str) -> Arc<ServerHandle> {
        let capacity = self
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .console_buffer_lines;
        let mut handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        handles
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(ServerHandle::new(id, capacity)))
            .clone()
    }

    pub fn forget(&self, id: &str) {
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub fn runtime(&self, id: &str) -> ServerRuntime {
        let handle = self.handle(id);
        let inner = handle.inner();
        ServerRuntime {
            status: inner.status.clone(),
            pid: inner.pid,
            uptime_ms: inner
                .started
                .filter(|_| inner.status == STATUS_RUNNING)
                .map(|s| s.elapsed().as_millis() as u64)
                .unwrap_or(0),
            cpu_percent: inner.cpu,
            memory_bytes: inner.memory,
            restarts: inner.restarts,
            started_at: inner.started_ms,
            last_exit_code: inner.last_exit,
            disk_bytes: inner.disk,
            console_lines: handle.log.len(),
            install_progress: inner.install_progress,
            install_step: inner.install_step.clone(),
        }
    }

    pub fn set_usage(&self, id: &str, cpu: f32, memory: u64, disk: u64) {
        if let Some(handle) = self
            .handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
        {
            let mut inner = handle.inner();
            inner.cpu = cpu;
            inner.memory = memory;
            if disk > 0 {
                inner.disk = disk;
            }
        }
    }

    pub fn set_disk(&self, id: &str, disk: u64) {
        if let Some(handle) = self
            .handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
        {
            handle.inner().disk = disk;
        }
    }

    pub fn running(&self) -> Vec<(String, u32)> {
        let handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        handles
            .values()
            .filter_map(|handle| {
                let inner = handle.inner();
                if inner.status == STATUS_RUNNING {
                    inner.pid.map(|pid| (handle.id.clone(), pid))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn console_snapshot(&self, id: &str, limit: usize) -> Vec<LogLine> {
        self.handle(id).log.snapshot(limit)
    }

    pub fn console_search(&self, id: &str, query: &str, limit: usize) -> Vec<LogLine> {
        self.handle(id).log.search(query, limit)
    }

    pub fn clear_console(&self, id: &str) {
        self.handle(id).log.clear();
        emit(
            &self.events,
            Event::new("console_cleared", Some(id.to_string()), json!({})),
        );
    }

    pub fn set_install(&self, id: &str, progress: Option<u8>, step: Option<String>) {
        let handle = self.handle(id);
        {
            let mut inner = handle.inner();
            inner.install_progress = progress;
            inner.install_step = step.clone();
        }
        emit(
            &self.events,
            Event::new(
                "install",
                Some(id.to_string()),
                json!({ "progress": progress, "step": step }),
            ),
        );
    }

    fn set_status(&self, handle: &ServerHandle, status: &str, exit: Option<i32>) {
        {
            let mut inner = handle.inner();
            inner.status = status.to_string();
            if status != STATUS_RUNNING && status != STATUS_STARTING {
                inner.started = None;
            }
        }
        let _ = self.db.set_server_status(&handle.id, status, exit);
        emit(
            &self.events,
            Event::new(
                "status",
                Some(handle.id.clone()),
                json!({ "status": status, "exitCode": exit }),
            ),
        );
    }

    fn system_log(&self, handle: &ServerHandle, text: impl Into<String>) {
        let line = handle.log.push("system", text.into());
        emit(
            &self.events,
            Event::new(
                "console",
                Some(handle.id.clone()),
                serde_json::to_value(&line).unwrap_or(json!({})),
            ),
        );
    }

    /// Mark persisted "running" servers as stopped after a panel restart.
    pub fn reconcile(&self) {
        let servers = match self.db.list_servers() {
            Ok(servers) => servers,
            Err(err) => {
                tracing::error!("could not reconcile server statuses: {err}");
                return;
            }
        };
        for server in servers {
            let stale = matches!(
                server.status.as_str(),
                STATUS_RUNNING | STATUS_STARTING | STATUS_STOPPING
            );
            let interrupted_install = server.status == STATUS_INSTALLING;
            if stale {
                let handle = self.handle(&server.id);
                self.set_status(&handle, STATUS_STOPPED, server.last_exit_code);
                self.system_log(
                    &handle,
                    "Previous session ended while WinPanel was not running; server is stopped.",
                );
                let _ = self.db.log_activity(
                    "info",
                    Some(&server.id),
                    None,
                    "Server marked stopped after panel restart",
                    None,
                );
            } else if interrupted_install {
                let _ = self.db.set_install_state(
                    &server.id,
                    server.installed,
                    Some("Installation was interrupted by a panel restart"),
                    if server.installed {
                        STATUS_STOPPED
                    } else {
                        STATUS_INSTALL_FAILED
                    },
                );
            }
        }
    }

    /// Start a server. The returned future is boxed so the supervisor can
    /// restart servers from its own monitor task without an infinitely sized
    /// future type.
    pub fn start(
        self: &Arc<Self>,
        server: Server,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let supervisor = Arc::clone(self);
        Box::pin(async move { supervisor.start_inner(server).await })
    }

    async fn start_inner(self: Arc<Self>, server: Server) -> Result<()> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(AppError::conflict("WinPanel is shutting down"));
        }
        let handle = self.handle(&server.id);
        {
            let mut inner = handle.inner();
            if matches!(inner.status.as_str(), STATUS_RUNNING | STATUS_STARTING) {
                return Err(AppError::conflict("Server is already running"));
            }
            if inner.status == STATUS_INSTALLING {
                return Err(AppError::conflict("Server is still installing"));
            }
            inner.status = STATUS_STARTING.to_string();
            inner.stop_requested = false;
            inner.last_exit = None;
            inner.install_progress = None;
            inner.install_step = None;
        }
        self.set_status(&handle, STATUS_STARTING, None);
        self.system_log(
            &handle,
            format!("Starting {} using {}", server.name, server.startup_exe),
        );

        if !server.installed {
            let message = "Server files are not installed yet. Run the installer first.";
            self.fail_start(&handle, message).await;
            return Err(AppError::conflict(message));
        }
        let install_dir = Path::new(&server.install_dir);
        if !install_dir.is_dir() {
            let message = format!("Server directory is missing: {}", server.install_dir);
            self.fail_start(&handle, &message).await;
            return Err(AppError::bad_request(message));
        }

        let executable = match resolve_executable(&server.startup_exe, install_dir) {
            Some(path) => path,
            None => {
                let message = format!(
                    "Executable \"{}\" was not found. Install the required runtime or set an absolute path in the server settings.",
                    server.startup_exe
                );
                self.fail_start(&handle, &message).await;
                return Err(AppError::bad_request(message));
            }
        };

        let mut command = Command::new(&executable);
        command
            .args(split_args(&server.startup_args))
            .current_dir(install_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for var in &server.env {
            if !var.key.is_empty() {
                command.env(&var.key, &var.value);
            }
        }
        // A predictable environment for game servers.
        command.env("SERVER_ID", &server.id);
        if let Some(port) = server.ports.first() {
            command.env("SERVER_PORT", port.port.to_string());
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            command
                .as_std_mut()
                .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.as_std_mut().pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                let message = format!("Could not start \"{}\": {err}", executable.display());
                self.fail_start(&handle, &message).await;
                return Err(AppError::internal(message));
            }
        };

        let pid = child.id();
        let guard = pid.and_then(|pid| {
            ProcessGuard::attach(
                pid,
                &Limits {
                    memory_mb: server.memory_mb,
                    cpu_limit_percent: server.cpu_limit_percent,
                    cpu_cores: server.cpu_cores.clone(),
                },
            )
        });

        if let Some(stdout) = child.stdout.take() {
            self.spawn_reader(handle.clone(), stdout, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            self.spawn_reader(handle.clone(), stderr, "stderr");
        }
        {
            let mut stdin = handle.stdin.lock().await;
            *stdin = child.stdin.take();
        }
        {
            let mut inner = handle.inner();
            inner.pid = pid;
            inner.started = Some(Instant::now());
            inner.started_ms = Some(now_ms());
            inner.guard = guard.map(Arc::new);
            if inner.guard.is_none() && pid.is_some() {
                // Job containment unavailable (already inside a restricted job);
                // PID-based termination is still in place.
                tracing::warn!("no job object for server {}", server.id);
            }
        }
        self.set_status(&handle, STATUS_RUNNING, None);
        self.system_log(
            &handle,
            format!(
                "Server running (pid {})",
                pid.map(|p| p.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
        );
        let _ = self.db.log_activity(
            "start",
            Some(&server.id),
            None,
            &format!("Started {}", server.name),
            None,
        );

        let supervisor = Arc::clone(&self);
        tokio::spawn(async move {
            supervisor.monitor(server, handle, child).await;
        });
        Ok(())
    }

    async fn fail_start(&self, handle: &ServerHandle, message: &str) {
        {
            let mut inner = handle.inner();
            inner.status = STATUS_STOPPED.to_string();
            inner.pid = None;
            inner.started = None;
        }
        let _ = self.db.set_server_status(&handle.id, STATUS_STOPPED, None);
        self.system_log(handle, message);
        emit(
            &self.events,
            Event::new(
                "status",
                Some(handle.id.clone()),
                json!({ "status": STATUS_STOPPED, "error": message }),
            ),
        );
        let _ = self
            .db
            .log_activity("error", Some(&handle.id), None, message, None);
    }

    fn spawn_reader<R>(&self, handle: Arc<ServerHandle>, reader: R, stream: &'static str)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(text)) => {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let line = handle.log.push(stream, text);
                        emit(
                            &events,
                            Event::new(
                                "console",
                                Some(handle.id.clone()),
                                serde_json::to_value(&line).unwrap_or(json!({})),
                            ),
                        );
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::debug!("console stream ended for {}: {err}", handle.id);
                        break;
                    }
                }
            }
        });
    }

    async fn monitor(self: Arc<Self>, server: Server, handle: Arc<ServerHandle>, mut child: Child) {
        let wait_result = child.wait().await;
        let exit_code = wait_result.as_ref().ok().and_then(|status| status.code());
        let ran_for = {
            let inner = handle.inner();
            inner.started.map(|s| s.elapsed()).unwrap_or_default()
        };
        let stop_requested = {
            let mut inner = handle.inner();
            inner.pid = None;
            inner.started = None;
            let requested = inner.stop_requested;
            inner.stop_requested = false;
            drop(inner.guard.take());
            requested
        };
        {
            let mut stdin = handle.stdin.lock().await;
            *stdin = None;
        }
        if let Err(err) = &wait_result {
            tracing::error!("failed waiting for server {}: {err}", server.id);
        }

        let shutting_down = self.shutting_down.load(Ordering::SeqCst);
        let crashed =
            !stop_requested && !shutting_down && exit_code.map(|code| code != 0).unwrap_or(true);

        {
            let mut inner = handle.inner();
            inner.status = if crashed {
                STATUS_CRASHED
            } else {
                STATUS_STOPPED
            }
            .to_string();
            inner.last_exit = exit_code;
            inner.cpu = 0.0;
            inner.memory = 0;
            if !crashed {
                inner.restarts = 0;
            }
        }
        let _ = self.db.set_server_status(
            &handle.id,
            if crashed {
                STATUS_CRASHED
            } else {
                STATUS_STOPPED
            },
            exit_code,
        );
        emit(
            &self.events,
            Event::new(
                "status",
                Some(handle.id.clone()),
                json!({
                    "status": if crashed { STATUS_CRASHED } else { STATUS_STOPPED },
                    "exitCode": exit_code
                }),
            ),
        );

        match exit_code {
            Some(code) if crashed => {
                self.system_log(
                    &handle,
                    format!("Server stopped unexpectedly (exit code {code})"),
                );
                let _ = self.db.log_activity(
                    "crash",
                    Some(&server.id),
                    None,
                    &format!("{} crashed with exit code {code}", server.name),
                    Some(&format!("uptime {}s", ran_for.as_secs())),
                );
            }
            Some(code) => {
                self.system_log(&handle, format!("Server stopped (exit code {code})"));
                let _ = self.db.log_activity(
                    "stop",
                    Some(&server.id),
                    None,
                    &format!("Stopped {}", server.name),
                    None,
                );
            }
            None => {
                self.system_log(&handle, "Server process ended");
            }
        }

        if crashed && server.auto_restart && !shutting_down {
            let attempt = {
                let mut inner = handle.inner();
                if ran_for >= STABLE_UPTIME {
                    inner.restarts = 0;
                }
                inner.restarts += 1;
                inner.restarts
            };
            if attempt <= server.auto_restart_max {
                self.system_log(
                    &handle,
                    format!(
                        "Automatic restart {attempt}/{} in {}s",
                        server.auto_restart_max,
                        server.auto_restart_delay_ms / 1000
                    ),
                );
                let supervisor = Arc::clone(&self);
                let server_clone = server.clone();
                let delay = server.auto_restart_delay_ms.clamp(500, 300_000);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    if supervisor.shutting_down.load(Ordering::SeqCst) {
                        return;
                    }
                    let mut restarts = 0;
                    if let Some(existing) = supervisor
                        .handles
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&server_clone.id)
                        .cloned()
                    {
                        restarts = existing.inner().restarts;
                    }
                    match Box::pin(supervisor.start(server_clone.clone())).await {
                        Ok(()) => {
                            if let Some(handle) = supervisor
                                .handles
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get(&server_clone.id)
                                .cloned()
                            {
                                handle.inner().restarts = restarts;
                            }
                        }
                        Err(err) => {
                            tracing::warn!("automatic restart failed: {err}");
                            let _ = supervisor.db.log_activity(
                                "error",
                                Some(&server_clone.id),
                                None,
                                "Automatic restart failed",
                                Some(&err.to_string()),
                            );
                        }
                    }
                });
            } else {
                self.system_log(
                    &handle,
                    format!(
                        "Restart limit reached ({}/{}) - server will stay offline",
                        server.auto_restart_max, server.auto_restart_max
                    ),
                );
            }
        }
    }

    pub async fn stop(&self, server: &Server, force: bool) -> Result<()> {
        let handle = self.handle(&server.id);
        let (was_active, timeout_ms, stop_command, guard) = {
            let mut inner = handle.inner();
            if !matches!(
                inner.status.as_str(),
                STATUS_RUNNING | STATUS_STARTING | STATUS_STOPPING
            ) {
                return Ok(());
            }
            inner.stop_requested = true;
            if !force {
                inner.status = STATUS_STOPPING.to_string();
            }
            (
                true,
                server.stop_timeout_ms.max(2_000),
                server.stop_command.clone(),
                inner.guard.clone(),
            )
        };
        if !was_active {
            return Ok(());
        }
        if force {
            self.system_log(&handle, "Force terminating server");
            if let Some(guard) = &guard {
                guard.terminate();
            }
            self.set_status(&handle, STATUS_STOPPING, None);
            return Ok(());
        }

        self.set_status(&handle, STATUS_STOPPING, None);
        let mut wrote = false;
        if !stop_command.trim().is_empty() {
            let mut stdin = handle.stdin.lock().await;
            if let Some(pipe) = stdin.as_mut() {
                let payload = format!("{}\n", stop_command.trim());
                if pipe.write_all(payload.as_bytes()).await.is_ok() {
                    let _ = pipe.flush().await;
                    wrote = true;
                }
            }
        }
        if wrote {
            self.system_log(
                &handle,
                format!("Sent \"{}\" to the server console", stop_command.trim()),
            );
        } else {
            self.system_log(
                &handle,
                "No stop command is configured; terminating the process tree",
            );
        }

        // Without a stop command there is nothing to wait for: give the process a
        // short grace period rather than blocking for the full graceful timeout.
        let grace_ms = if wrote {
            timeout_ms
        } else {
            timeout_ms.min(1_500)
        };
        let handle_clone = handle.clone();
        let log = handle.log.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let deadline = Instant::now() + Duration::from_millis(grace_ms);
            while Instant::now() < deadline {
                if !handle_clone.is_active() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if handle_clone.is_active() {
                let line = log.push(
                    "system",
                    "Stop timeout reached; terminating the process tree".to_string(),
                );
                emit(
                    &events,
                    Event::new(
                        "console",
                        Some(handle_clone.id.clone()),
                        serde_json::to_value(&line).unwrap_or(json!({})),
                    ),
                );
                if let Some(guard) = &guard {
                    guard.request_stop();
                    guard.terminate();
                }
            }
        });
        Ok(())
    }

    pub async fn restart(self: &Arc<Self>, server: Server, force: bool) -> Result<()> {
        self.stop(&server, force).await?;
        let handle = self.handle(&server.id);
        let supervisor = Arc::clone(self);
        tokio::spawn(async move {
            let deadline = Instant::now() + Duration::from_secs(45);
            while Instant::now() < deadline {
                if !handle.is_active() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if handle.is_active() {
                if let Some(guard) = handle.inner().guard.clone() {
                    guard.terminate();
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if let Err(err) = supervisor.start(server.clone()).await {
                tracing::warn!("restart of {} failed: {err}", server.id);
            }
        });
        Ok(())
    }

    pub async fn write_input(&self, server_id: &str, command: &str) -> Result<()> {
        if command.len() > 4096 {
            return Err(AppError::bad_request("Command is too long"));
        }
        if command.contains('\n') || command.contains('\r') {
            return Err(AppError::bad_request("Commands must be a single line"));
        }
        let handle = self.handle(server_id);
        let mut stdin = handle.stdin.lock().await;
        let pipe = stdin
            .as_mut()
            .ok_or_else(|| AppError::conflict("Server is not accepting console input"))?;
        pipe.write_all(command.as_bytes())
            .await
            .map_err(|e| AppError::internal(format!("Could not write to server console: {e}")))?;
        pipe.write_all(b"\n")
            .await
            .map_err(|e| AppError::internal(format!("Could not write to server console: {e}")))?;
        pipe.flush()
            .await
            .map_err(|e| AppError::internal(format!("Could not flush server console: {e}")))?;
        drop(stdin);

        let line = handle.log.push("system", format!("> {command}"));
        emit(
            &self.events,
            Event::new(
                "console",
                Some(server_id.to_string()),
                serde_json::to_value(&line).unwrap_or(json!({})),
            ),
        );
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let servers = match self.db.list_servers() {
            Ok(servers) => servers,
            Err(err) => {
                tracing::error!("cannot list servers for shutdown: {err}");
                return;
            }
        };
        let active: Vec<Server> = servers
            .into_iter()
            .filter(|server| self.handle(&server.id).is_active())
            .collect();
        if active.is_empty() {
            return;
        }
        tracing::info!("stopping {} running server(s)", active.len());
        for server in &active {
            if let Err(err) = self.stop(server, false).await {
                tracing::warn!("stop {} failed: {err}", server.id);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            if active.iter().all(|s| !self.handle(&s.id).is_active()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        for server in &active {
            let handle = self.handle(&server.id);
            if handle.is_active() {
                tracing::warn!("force terminating {} during shutdown", server.id);
                if let Some(guard) = handle.inner().guard.clone() {
                    guard.terminate();
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
