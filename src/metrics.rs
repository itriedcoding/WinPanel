//! Real system monitoring built on `sysinfo`.
//!
//! A single low-frequency sampler refreshes CPU, memory, network and process
//! data, aggregates per-server usage over each server's process tree, and keeps
//! a short in-memory history for the dashboard charts. Nothing is polled by the
//! browser, so an idle panel costs an idle process and no extra CPU.

use crate::process::Supervisor;
use crate::util::now_ms;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, System};

const HISTORY_POINTS: usize = 240;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    pub received_per_sec: u64,
    pub transmitted_per_sec: u64,
    pub total_received: u64,
    pub total_transmitted: u64,
    pub interfaces: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcUsage {
    pub cpu: f32,
    pub memory: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub ts: i64,
    pub cpu: f32,
    pub memory_used: u64,
    pub net_rx: u64,
    pub net_tx: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub cpu_percent: f32,
    pub cpu_cores: usize,
    pub cpu_brand: String,
    pub memory_total: u64,
    pub memory_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub disks: Vec<DiskInfo>,
    pub network: NetworkInfo,
    pub uptime_secs: u64,
    pub hostname: String,
    pub os: String,
    pub panel_memory: u64,
    pub servers: HashMap<String, ProcUsage>,
    pub sampled_at: i64,
    pub process_count: usize,
}

pub struct Metrics {
    snapshot: RwLock<SystemSnapshot>,
    history: Mutex<VecDeque<HistoryPoint>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(SystemSnapshot::default()),
            history: Mutex::new(VecDeque::with_capacity(HISTORY_POINTS)),
        }
    }

    pub fn snapshot(&self) -> SystemSnapshot {
        self.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn history(&self) -> Vec<HistoryPoint> {
        self.history
            .lock()
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Start the background sampler. Returns a handle that stops it when aborted.
pub fn spawn_sampler(
    metrics: Arc<Metrics>,
    supervisor: Arc<Supervisor>,
    interval_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut system = System::new();
        system.refresh_cpu_usage();
        system.refresh_memory();
        system.refresh_processes(ProcessesToUpdate::All, true);

        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();

        let mut prev_net_rx = network_total(&networks).0;
        let mut prev_net_tx = network_total(&networks).1;
        let mut prev_disk = disk_totals(&disks);
        let mut prev_at = Instant::now();

        let mut ticker =
            tokio::time::interval(Duration::from_millis(interval_ms.clamp(1000, 60_000)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;

            // Drive the CPU delta from the previous sample.
            system.refresh_cpu_usage();
            system.refresh_memory();
            system.refresh_processes(ProcessesToUpdate::All, true);
            disks.refresh(true);
            networks.refresh(true);

            let now = Instant::now();
            let elapsed = now.duration_since(prev_at).as_secs_f64().max(0.25);
            prev_at = now;

            let (net_rx_total, net_tx_total) = network_total(&networks);
            let rx_per_sec = net_rx_total.saturating_sub(prev_net_rx) as f64 / elapsed;
            let tx_per_sec = net_tx_total.saturating_sub(prev_net_tx) as f64 / elapsed;
            prev_net_rx = net_rx_total;
            prev_net_tx = net_tx_total;

            let disk_totals_now = disk_totals(&disks);
            let disk_info = collect_disks(&disks, &prev_disk, elapsed);
            prev_disk = disk_totals_now;

            let parent_map: HashMap<u32, u32> = system
                .processes()
                .iter()
                .filter_map(|(pid, process)| {
                    process
                        .parent()
                        .map(|parent| (pid.as_u32(), parent.as_u32()))
                })
                .collect();

            let mut per_server: HashMap<String, ProcUsage> = HashMap::new();
            for (server_id, root_pid) in supervisor.running() {
                let mut cpu = 0.0f32;
                let mut memory = 0u64;
                for (pid, process) in system.processes() {
                    let pid_u32 = pid.as_u32();
                    if pid_u32 == root_pid || is_descendant(pid_u32, root_pid, &parent_map) {
                        cpu += process.cpu_usage();
                        memory = memory.saturating_add(process.memory());
                    }
                }
                let usage = ProcUsage { cpu, memory };
                supervisor.set_usage(&server_id, usage.cpu, usage.memory, 0);
                per_server.insert(server_id, usage);
            }

            let panel_memory = system
                .process(Pid::from_u32(std::process::id()))
                .map(|process| process.memory())
                .unwrap_or(0);

            let snapshot = SystemSnapshot {
                cpu_percent: system.global_cpu_usage(),
                cpu_cores: system.cpus().len(),
                cpu_brand: system
                    .cpus()
                    .first()
                    .map(|cpu| cpu.brand().trim().to_string())
                    .unwrap_or_else(|| "Unknown CPU".to_string()),
                memory_total: system.total_memory(),
                memory_used: system.used_memory(),
                swap_total: system.total_swap(),
                swap_used: system.used_swap(),
                disks: disk_info,
                network: NetworkInfo {
                    received_per_sec: rx_per_sec as u64,
                    transmitted_per_sec: tx_per_sec as u64,
                    total_received: net_rx_total,
                    total_transmitted: net_tx_total,
                    interfaces: networks.len(),
                },
                uptime_secs: System::uptime(),
                hostname: System::host_name().unwrap_or_else(|| "localhost".to_string()),
                os: System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string()),
                panel_memory,
                servers: per_server,
                sampled_at: now_ms(),
                process_count: system.processes().len(),
            };

            let point = HistoryPoint {
                ts: snapshot.sampled_at,
                cpu: snapshot.cpu_percent,
                memory_used: snapshot.memory_used,
                net_rx: snapshot.network.received_per_sec,
                net_tx: snapshot.network.transmitted_per_sec,
            };
            if let Ok(mut history) = metrics.history.lock() {
                history.push_back(point);
                while history.len() > HISTORY_POINTS {
                    history.pop_front();
                }
            }
            if let Ok(mut shared) = metrics.snapshot.write() {
                *shared = snapshot;
            }
        }
    })
}

fn network_total(networks: &Networks) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    for data in networks.values() {
        rx = rx.saturating_add(data.total_received());
        tx = tx.saturating_add(data.total_transmitted());
    }
    (rx, tx)
}

fn disk_totals(disks: &Disks) -> HashMap<String, (u64, u64)> {
    let mut map = HashMap::new();
    for disk in disks.list() {
        let usage = disk.usage();
        map.insert(
            disk.mount_point().to_string_lossy().to_string(),
            (usage.total_read_bytes, usage.total_written_bytes),
        );
    }
    map
}

fn collect_disks(
    disks: &Disks,
    previous: &HashMap<String, (u64, u64)>,
    elapsed: f64,
) -> Vec<DiskInfo> {
    let mut out = Vec::new();
    for disk in disks.list() {
        let mount = disk.mount_point().to_string_lossy().to_string();
        let usage = disk.usage();
        let (prev_read, prev_write) = previous.get(&mount).copied().unwrap_or((0, 0));
        out.push(DiskInfo {
            name: disk.name().to_string_lossy().to_string(),
            mount,
            file_system: disk.file_system().to_string_lossy().to_string(),
            total: disk.total_space(),
            available: disk.available_space(),
            removable: disk.is_removable(),
            read_bytes_per_sec: (usage.total_read_bytes.saturating_sub(prev_read) as f64 / elapsed)
                as u64,
            write_bytes_per_sec: (usage.total_written_bytes.saturating_sub(prev_write) as f64
                / elapsed) as u64,
        });
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

fn is_descendant(pid: u32, root: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut current = pid;
    for _ in 0..32 {
        match parents.get(&current) {
            Some(&parent) if parent == root => return true,
            Some(&parent) if parent == current => return false,
            Some(&parent) => current = parent,
            None => return false,
        }
    }
    false
}
