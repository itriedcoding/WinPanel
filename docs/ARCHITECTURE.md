# Architecture

WinPanel is a single Rust binary that serves an embedded web UI and supervises child
processes. This document explains how the pieces fit together, which is useful before
modifying the code.

## Runtime shape

```
main.rs
 ├─ CLI parsing, data directory, config.json
 ├─ tracing to stdout + logs/winpanel.log
 ├─ SQLite (WAL)  ── Db
 ├─ AppState ─┬─ Supervisor ── child processes, job objects, console buffers
 │            ├─ Metrics    ── sysinfo sampler + history ring
 │            ├─ BackupManager (scheduler + zip/restore)
 │            ├─ UpdateManager (GitHub Releases)
 │            ├─ Template registry (built-in + user JSON)
 │            └─ event bus (tokio broadcast)
 ├─ axum router: /api/* JSON + WebSockets, everything else from rust-embed
 └─ background tasks: metrics, storage usage, backups, housekeeping, update checks
```

Graceful shutdown: `Ctrl+C`/SIGTERM stops the HTTP server, then `Supervisor::shutdown`
sends each server's stop command, waits, and terminates remaining process trees before the
process exits.

## Concurrency model

- Tokio multithreaded runtime; all HTTP handlers are async.
- `Db` owns one `rusqlite::Connection` behind a `Mutex`. Writes are short and serialized;
  WAL mode keeps readers fast.
- Filesystem-heavy work (directory scans, zip creation, restore, server clones, extraction)
  runs on `spawn_blocking` so the reactor never stalls.
- `Supervisor` keeps an `Arc<ServerHandle>` per server: console buffer, stdin handle and
  runtime state. Status changes are published on a `broadcast::Sender<Event>`; the UI
  receives them through two WebSockets (per-server console, global events).
- Metrics sampling is one task with one timer; the browser polls the dashboard, but the
  server never fans out per-client refreshes.

## Process lifecycle

1. `start` validates the executable (`runtime::resolve_executable`), builds a
   `tokio::process::Command` with arguments from `util::split_args` (never a shell), sets the
   working directory and environment, and spawns with piped stdio.
2. On Windows the child is created with `CREATE_NO_WINDOW` and assigned to a Job Object:
   - `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — children die with the panel,
   - `JOB_OBJECT_LIMIT_PROCESS_MEMORY` — memory cap in bytes,
   - CPU rate control (`JOBOBJECT_CPU_RATE_CONTROL_INFORMATION`) — hard percentage cap,
   - `SetProcessAffinityMask` — selected cores.
3. Reader tasks stream stdout/stderr line by line into the bounded `LogBuffer` and the
   event bus.
4. A monitor task awaits the child. Exit status, uptime and whether a stop was requested
   determine `stopped` vs `crashed`; crashes are logged with the exit code and can trigger an
   automatic restart with backoff and an attempt limit.
5. Graceful stop writes `stopCommand` to stdin and waits up to the configured timeout, then
   terminates the process tree. Force stop terminates immediately.
6. On panel startup, any persisted `running` status is reconciled to `stopped`, so the UI
   never shows a phantom process.

## Database schema

| Table | Purpose |
| --- | --- |
| `users` | Accounts, Argon2id hashes, role, disabled flag, last login |
| `sessions` | SHA-256 of the session token, CSRF secret, expiry |
| `servers` | Full server configuration and last known status |
| `server_access` | `(user, server) -> view/console/files/manage` |
| `activity` | Audit trail used by the dashboard and activity views |
| `backups` | Archive metadata, checksum and status |
| `settings` | One JSON row with application preferences |

Migrations are applied with `PRAGMA user_version`; the schema is created idempotently.

## HTTP surface

- `POST /api/auth/setup|login|logout`, `GET /api/auth/status|session`, `POST /api/auth/password`
- `GET /api/dashboard`, `/api/system`, `/api/system/history`, `/api/system/runtimes`, `/api/system/info`
- `GET|POST /api/servers`, `GET|PATCH|DELETE /api/servers/:id`
- `POST /api/servers/:id/action` (`start|stop|restart|kill|rename|clone|delete`)
- `POST /api/servers/:id/install`
- `GET /api/servers/:id/console`, `POST .../console/input`, `POST .../console/clear`
- `WS /api/servers/:id/ws` (console), `WS /api/events` (status/install/backup)
- `GET /api/servers/:id/files`, `content`, `search`, `download`; `PUT .../upload`;
  `POST mkdir|touch|rename|copy|delete|chmod`
- `GET|POST /api/servers/:id/backups`, `DELETE .../backups/:id`, `POST .../backups/:id/restore`
- `GET|POST /api/users`, `PATCH|DELETE /api/users/:id`, `POST /api/users/:id/access`
- `GET /api/templates`, `POST /api/templates/reload`
- `GET|PUT /api/settings`
- `GET /api/updates/check`, `POST /api/updates/download`, `POST /api/updates/install`

Errors are uniform JSON: `{ "error": "forbidden", "message": "..." }` with appropriate HTTP
status codes. Mutations require `X-CSRF-Token`; a mismatch or missing header is rejected
before the handler runs.

## Front end

`web/` is plain ES modules, CSS and SVG icons. There is no bundler, no framework and no
node_modules. The router is hash-based; each view returns a cleanup function that closes
sockets and timers when you navigate away. Live updates arrive through WebSockets, so the
dashboard and server status stay current without polling loops.

Design tokens live at the top of `web/css/app.css`: near-black background, charcoal
surfaces, one accent (white), one danger hue for errors, 8 px radii and a 13.5 px base font.

## Extending

- **New template** — add JSON (see `docs/TEMPLATES.md`). No code changes.
- **New install method** — add a branch in `install.rs` and a variant to the validation list
  in `templates.rs`.
- **New API endpoint** — add a handler module under `src/api/`, register it in
  `api::router`, and use `load_server` for permission checks.
- **New UI view** — add `web/js/views/<name>.js`, wire it into `app.js` routing and the
  sidebar.
