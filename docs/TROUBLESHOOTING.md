# Troubleshooting

Start with the log: `%APPDATA%\WinPanel\logs\winpanel.log` (or
`<data-dir>\logs\winpanel.log`). Each failure the panel detects is also written to the
server console and to **Settings → Activity log**.

---

## The panel does not start

**"Cannot bind 127.0.0.1:8787: ... address already in use"**

Another process is using the port. Find it:

```powershell
Get-NetTCPConnection -LocalPort 8787 | Select-Object OwningProcess
Get-Process -Id <pid>
```

Change the port in `config.json` (`"listen": "127.0.0.1:8790"`) and start again.

**"Refusing to bind 0.0.0.0:8787: remote access is disabled"**

WinPanel requires both a non-loopback address and explicit consent:

```json
{ "listen": "0.0.0.0:8787", "allowRemote": true }
```

Or start with `--allow-remote`. This is deliberate: a panel that silently binds to every
interface is a security incident waiting to happen.

**A console window flashes and closes**

Run the executable from `cmd` or PowerShell to read the error:

```powershell
.\WinPanel.exe
```

**Antivirus quarantines the executable**

Unsigned single-file utilities are sometimes flagged heuristically. Verify the SHA-256
against the release's `SHA256SUMS.txt`, then allow the file in your AV product. Building
from source is always an option.

---

## Sign-in problems

**Locked out after failed attempts**

Wait five minutes; the lockout is per username and expires automatically.

**"Session expired" repeatedly**

Sessions are stored in SQLite and expire according to **Settings → Session lifetime**. If the
system clock jumps or the disk is full, sessions can fail. Check the log for database
errors and confirm free space on the data drive.

**Cannot reset a forgotten password**

There is no email flow. With access to the machine, stop WinPanel, open
`winpanel.db` with any SQLite tool and remove the row from `users` for the forgotten
account (or delete all users to return to the setup screen). Then start WinPanel again and
use the setup screen. Removing the last administrator returns the panel to first-run setup.

---

## Creating a server

**"Java runtime is required but was not found"**

Install Java 21+ (Eclipse Temurin is a good default), make sure `java -version` works in a
new terminal, then press **Reinstall** on the server. WinPanel also searches
`JAVA_HOME`, `%ProgramFiles%\Java`, Adoptium and Chocolatey paths, so a fresh install is
usually picked up without changing `PATH`.

**"Port 25565/tcp is already assigned to ..."**

WinPanel blocks duplicate ports across servers. Change the port or delete the other server.

**"Cannot create the server directory"**

The data directory is not writable, the disk is full, or antivirus blocked the write. Pick
another default install directory in **Settings** (absolute path) and try again.

**Installation fails immediately**

Open the server and read the banner and console. The error is the upstream message, for
example:

- HTTP 403/404 from a download host — the release URL moved or the network blocks it;
- checksum mismatch — the download was corrupted, retry;
- "Minecraft version X does not exist" — use `latest` or a valid release id.

Failures are recorded on the server, so pressing **Reinstall** retries cleanly.

---

## Running servers

**Start says the executable was not found**

`startupExe` must be one of:

- a command on `PATH` (`java`, `node`, `python`),
- a file inside the server directory (`server.exe`, `./bin/app`),
- an absolute path (`C:\Java\bin\java.exe`).

For generic templates, upload the executable first, then set its name in Settings.

**Server exits immediately**

The console shows the child's stderr and the exit code. Typical causes: missing
`eula=true` for Minecraft, a world saved by a newer version, a Java version that is too old,
or a missing dependency. The exit code is also stored and shown in the server sidebar.

**Stop does not work**

Servers stop by writing the configured stop command to stdin and waiting for the timeout.
Programs that do not read stdin need a different command (or an empty one, which skips
straight to termination). Use **Force terminate** to kill the tree immediately.

**A server keeps restarting**

Automatic restarts are limited by `max restarts`; each crash increments the counter and a
stable run resets it. Open the console to see the crash reason, then disable automatic
restarts while debugging.

**Processes survive after the panel is closed**

Job objects normally kill the tree when the panel exits. If WinPanel itself runs inside
another job object that forbids nesting (some CI or sandbox environments), assignment fails
and the panel logs a warning; termination then uses `taskkill /T`. In that case, check
Task Manager for leftover processes and run the panel outside the restrictive job.

**Memory limit does not seem to apply**

The limit is a Windows Job Object process memory cap and is enforced in release builds on
Windows. Java reserves a large virtual address space; use `-Xmx` in the startup arguments as
well and keep the job limit slightly above the JVM heap.

---

## Files

**Upload fails at 100%**

The storage quota (`Storage limit`) or the global upload cap
(**Settings → Maximum upload size**) was reached. Increase it or free space. The partial
file is removed automatically.

**A file cannot be edited**

Files above 2 MB and binary files are not editable in the browser; download them instead.

**Access denied while browsing**

Your account's access level for that server is `view` or `console`. Ask an administrator
for `files` or `manage`.

---

## Backups

**Restore refuses to start**

Stop the server first. Restoring swaps the entire server directory, which cannot happen
while a process holds files open.

**"Backup archive is corrupted (checksum mismatch)"**

The zip changed after it was created (interrupted copy, disk error, antivirus). Delete the
backup and create a new one; do not rely on the damaged archive.

**Backups are not created on schedule**

Scheduled backups have a default interval of 24 hours and only run when the panel is
running. Enable them per server (Settings → Backups) or globally, and check the activity log
for failures. A backup that is already running blocks a second one for the same server.

---

## Updates

**"The latest GitHub release is a draft or pre-release"**

Only normal releases are offered. Try again after the release is published.

**"Update failed checksum verification"**

The download was corrupted or intercepted. Retry; if it persists, download the executable
manually from the releases page.

**The panel did not restart after installing an update**

The update script waits for the current process to exit, moves the executable, and starts
the new one. If it was blocked (antivirus, permissions on the folder), start
`WinPanel.exe` manually; the new binary is already in place. `apply-update.log` in the data
directory records the steps.

---

## Monitoring

**CPU or disk numbers look wrong**

Windows counters need one sampling interval to compute a delta; the first data points after
starting the panel are zero. Disk read/write rates are only reported when the OS exposes
them for the volume.

**Panel memory grows over time**

Console buffers are bounded (`console buffer lines`, default 2000 per server) and metrics
history is a fixed 240-sample ring. If memory grows steadily, capture a memory snapshot and
open an issue with the number of servers and console activity.
