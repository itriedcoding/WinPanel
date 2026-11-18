# WinPanel

**A lightweight, Windows-native server management panel written in Rust.**

WinPanel is a small alternative to Pterodactyl Panel for people who just want to run game
servers and applications on Windows. No Linux, no Docker, no WSL, no Node.js runtime, no
database server. One executable starts a local management interface, supervises real
processes and stays out of the way.

```
WinPanel.exe  →  http://127.0.0.1:8787  →  your servers
```

---

## Highlights

- **Single self-contained executable.** The web interface, SQLite storage and server
  supervisor are compiled into one native binary. Nothing else is installed.
- **Real process management on Windows.** Servers run inside Win32 Job Objects, so the
  whole process tree is terminated together, memory limits are enforced by the OS and
  nothing survives a panel crash.
- **Live consoles.** WebSocket streams of stdout/stderr with timestamps, error
  highlighting, command input, search, history and auto-scroll.
- **Working installers.** Minecraft Java (Mojang), Paper, Bedrock, FiveM, Java, Node.js,
  Python and custom executables — each resolved against the project's current official API.
- **A real file manager.** Browse, upload (drag and drop), download, edit, rename, copy,
  delete and search inside the server directory — sandboxed against path traversal.
- **Backups that restore.** Zip archives with SHA-256 checksums, scheduled retention and a
  safe staging/rollback restore.
- **Actual system metrics.** CPU, memory, disks, network throughput and per-server
  process usage from `sysinfo`, sampled on a low-frequency background task.
- **Accounts and permissions.** Argon2id passwords, session cookies, CSRF protection and
  per-server access levels.
- **Loopback by default.** The API is bound to `127.0.0.1` and refuses a non-loopback bind
  until you explicitly allow it.
- **Built-in updater.** Checks GitHub Releases, verifies the SHA-256 digest and swaps the
  executable without touching your data.

---

## Features

### Dashboard
Total / running / offline / failing servers, CPU and memory load, network throughput,
per-volume disk usage, a 12-minute load chart, recently managed servers and a live
activity feed. Every value comes from the host or from managed processes — there are no
demo numbers anywhere in the code.

### Servers
Create, rename, clone, reinstall, start, stop, restart, force-terminate and delete.
Per server you configure:

| Setting | Notes |
| --- | --- |
| Name, description | Used for the folder name and in the UI |
| Template | Determines the installer and defaults |
| Startup executable | Absolute path, file inside the server dir, or a `PATH` command |
| Startup arguments | Quote-aware tokenizer; **no shell is involved** |
| Environment variables | Injected into the child process |
| Memory limit | Enforced with a Job Object `ProcessMemoryLimit` |
| CPU cores / CPU cap | Processor affinity and a hard CPU rate cap |
| Storage limit | Advisory quota enforced for uploads and edits |
| Ports | TCP/UDP mappings with cross-server conflict detection |
| Stop command + timeout | Written to stdin before escalating |
| Automatic restarts | Attempt limit, delay and crash-only behaviour |
| Scheduled backups | Interval and retention |

Servers marked `running` in the database are reconciled to `stopped` on startup, so the
panel never lies about a process that no longer exists.

### Console
Full stdout/stderr capture with bounded memory (configurable line count), ANSI/§ colour
code stripping, auto-classification of warnings and errors, incremental live streaming
over a WebSocket, command history stored in the browser, copy, clear and output filtering.

### Files
Directory listing with sizes and timestamps, breadcrumbs, text editor for files up to 2 MB,
upload streaming with size and quota enforcement, downloads with correct
`Content-Disposition`, rename/copy/delete/mkdir, read-only toggling and recursive name
search. Every path goes through a canonicalising sandbox check.

### Templates
Built-in templates: Minecraft Java (vanilla), Paper, Bedrock, FiveM, generic Java, Node.js,
Python and custom executables. Templates are data, not code — drop a JSON file into
`<data>/templates/` to add your own; see [docs/TEMPLATES.md](docs/TEMPLATES.md).

### Backups
On-demand and scheduled zip backups with progress events, SHA-256 checksums, automatic
retention pruning and restore that stages files first, swaps directories atomically and
rolls back if the swap fails. Restores require the server to be stopped.

### Users and security
First-run administrator setup, additional accounts with `admin`/`user` roles and
per-server levels (`view` → `console` → `files` → `manage`). Argon2id hashing, hashed
session tokens, `HttpOnly` + `SameSite=Strict` cookies, per-session CSRF tokens, origin
checks on mutations, login throttling, input validation and uniform JSON errors.

### Monitoring
A single sampler refreshes CPU, memory, swap, disks (with read/write throughput) and
network counters, then aggregates CPU and memory across each server's process tree.
Idle WinPanel uses a small fraction of one core; sampling costs are bounded by one timer.

### Updates
Checks `itriedcoding/WinPanel` releases on GitHub, shows release notes, downloads the new
`.exe`, verifies its digest and size, then replaces the running binary through a helper
script that waits for the panel to exit. Server data and settings are untouched.

---

## System requirements

- **Windows 10 or 11 (x64)**. Windows Server 2019+ works as well.
- No runtime prerequisites. WinPanel does not need .NET, Java, Node.js or a browser
  *engine* to start; it opens your default browser (or Edge/Chrome in app mode) for the UI.
- A modern browser to use the interface: Edge, Chrome or Firefox.
- Disk space for the panel itself is a few megabytes. Servers need their own space.
- Optional runtimes per template: Java 21+ for modern Minecraft, Node.js 18+ for Node
  applications, Python 3.10+ for Python applications. WinPanel detects them and refuses to
  install a template whose runtime is missing, with a clear message.

The panel also builds and runs on Linux and macOS, which is handy for development. Windows
Job Object containment and memory limits are Windows-only by design.

---

## Installation

1. Download `WinPanel.exe` from the [Releases](https://github.com/itriedcoding/WinPanel/releases)
   page.
2. Put it in a folder you control, for example `C:\WinPanel\WinPanel.exe`.
3. Run it. A console window shows the local URL and the app window opens automatically.
4. Create the administrator account on the first-run screen.
5. Create a server, pick a template, and press **Create and install**.

Data is stored in `%APPDATA%\WinPanel`:

```
%APPDATA%\WinPanel\
├─ config.json          bootstrap configuration (listen address, remote access)
├─ winpanel.db          SQLite database (users, servers, settings, backups, activity)
├─ logs\winpanel.log    rotating application log
├─ servers\             server directories by default
├─ backups\             backup archives
├─ templates\           user-supplied templates (*.json)
└─ tmp\                 downloads and update staging
```

Use `--data-dir` to keep everything on another drive:

```powershell
.\WinPanel.exe --data-dir "D:\WinPanel"
```

### Command line

```
winpanel [options]

  --listen <addr>     Bind address (default 127.0.0.1:8787)
  --data-dir <path>   Data directory for database, servers, backups and logs
  --allow-remote      Allow binding a non-loopback address
  --no-browser        Do not open the interface automatically
  -V, --version       Print the version
  -h, --help          Print help
```

### Remote access (optional)

WinPanel is local-only until you change two things:

```json
// %APPDATA%\WinPanel\config.json
{
  "listen": "0.0.0.0:8787",
  "allowRemote": true,
  "openBrowser": true,
  "logLevel": "info"
}
```

Both the address **and** the explicit `allowRemote` flag are required; the panel exits with
a diagnostic message otherwise. Add a firewall rule for the port and use strong passwords.
Traffic is plain HTTP — put a TLS reverse proxy in front of it if it leaves your LAN.

---

## Building from source

Requirements: [Rust](https://rustup.rs) stable 1.77 or newer and the MSVC build tools
("Desktop development with C++" in the Visual Studio Installer).

```powershell
git clone https://github.com/itriedcoding/WinPanel.git
cd WinPanel
cargo build --release
# binary: target\release\winpanel.exe
```

SQLite is compiled in (`rusqlite` bundled), so no external database is needed.

Useful commands:

```powershell
cargo run                 # debug build, serves the UI from web/ on disk
cargo test                # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release     # optimized single executable
```

`web/` is embedded into the binary at build time with `rust-embed`. In debug builds the
assets are read from disk, so front-end edits show up on refresh.

---

## Using WinPanel

1. **Overview** — system health and recent activity.
2. **Servers → New server** — choose a template, set name, port, memory and options.
   Installation runs in the background and streams progress into the server view.
3. **Console** — watch startup, type commands (Minecraft `stop`, `say`, …) and search output.
4. **Files** — upload plugins/mods/resources, edit `server.properties`, download logs.
5. **Backups** — create a snapshot before an update, restore it if something breaks.
6. **Settings** — resources, startup command, ports, environment, automation and danger
   zone actions (reinstall, clone, delete).
7. **Users** — give another account access to specific servers.
8. **Settings → Updates** — check, download and install new WinPanel versions.

The console sends whatever you type to the process stdin. If a program does not read stdin,
use its own commands or the stop command configured in server settings.

---

## Security model

- API bound to loopback unless `allowRemote` is explicitly enabled.
- Argon2id password hashing with per-password salts; plaintext passwords are never stored
  or logged.
- Session tokens are random 256-bit values stored only as SHA-256 hashes; cookies are
  `HttpOnly`, `SameSite=Strict` and `Path=/`.
- Every mutating request needs the per-session CSRF token, and browser `Origin` headers are
  compared with `Host`.
- Five failed sign-ins trigger a timed lockout.
- Role checks (admin) and server-level checks (`view`/`console`/`files`/`manage`) run
  server-side for every endpoint.
- File paths are normalised and canonicalised under the server root; `..`, absolute paths,
  drive letters and symlink escapes are rejected.
- Startup commands are executed directly with an argument vector: no `cmd.exe`, no shell
  interpolation, no command injection.
- Responses carry `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy` and a
  strict Content-Security-Policy.

See [docs/SECURITY.md](docs/SECURITY.md) for details and operational advice.

---

## Project structure

```
src/
├─ main.rs              CLI, logging, startup, graceful shutdown, background tasks
├─ api/                 HTTP handlers (auth, servers, files, backups, users, system, updates, ws)
├─ process/             supervisor, containment guard (Job Objects / POSIX), console buffers
├─ auth.rs              hashing, sessions, CSRF, permissions, login throttling
├─ db.rs                SQLite schema and queries
├─ install.rs           template installers (Mojang, Paper, Bedrock, FiveM, archives)
├─ templates.rs         template loading/validation and user templates
├─ backups.rs           zip creation, restore, retention, scheduler
├─ files.rs             sandboxed file operations
├─ metrics.rs           sysinfo sampling and per-server aggregation
├─ updates.rs           GitHub Releases checks, download, verification, apply
├─ runtime.rs           Java/Node/Python discovery and PATH resolution
└─ ...                  config, paths, errors, models and small utilities

web/                    embedded single-page interface (vanilla ES modules, no build step)
templates/builtin.json  built-in server templates
docs/                   installation, architecture, templates, security, troubleshooting
.github/workflows/      CI and Windows release automation
```

---

## Testing and quality gates

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

CI runs these on Linux and Windows for every push and pull request. Releases are built on
`windows-latest` by `.github/workflows/release.yml`, which also smoke-tests the produced
executable before publishing it with a SHA-256 checksum file.

To cut a release, bump the version in `Cargo.toml` and push a `v<version>` tag (or a branch
named `release/v<version>`). The workflow builds, tests, creates the GitHub release and
uploads `WinPanel.exe` and `SHA256SUMS.txt`.

Verify a download with:

```powershell
Get-FileHash .\WinPanel.exe -Algorithm SHA256
```

---

## Troubleshooting

Common problems and fixes are documented in
[docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md). Quick checks:

- **Port already in use** — set another `listen` value in `config.json`.
- **Cannot reach the panel** — it binds to loopback by default; see *Remote access*.
- **Server will not start** — open the console; missing Java/Node/Python produces an
  explicit message, as does a missing executable or a failed installation.
- **Installation failed** — the upstream API response is shown in the server banner and the
  activity log. Retry with **Reinstall** once the network issue is resolved.
- **Logs** — `%APPDATA%\WinPanel\logs\winpanel.log`.

---

## Reporting issues

Please include:

1. WinPanel version (Settings → About) and Windows build,
2. what you did, what you expected and what happened,
3. the relevant part of `winpanel.log`,
4. the server's console output for server-specific problems.

Open an issue at <https://github.com/itriedcoding/WinPanel/issues>. Include no secrets —
redact license keys, tokens and IP addresses.

---

## License

MIT — see [LICENSE](LICENSE).
