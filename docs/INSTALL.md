# Installing WinPanel

WinPanel is a single Windows executable. This guide covers a normal installation, a portable
installation on another drive, starting it automatically, and building it yourself.

---

## 1. Normal installation

1. Download `WinPanel.exe` from the [releases page](https://github.com/itriedcoding/WinPanel/releases).
2. Verify the download (optional but recommended):

   ```powershell
   Get-FileHash .\WinPanel.exe -Algorithm SHA256
   ```

   Compare the result with `SHA256SUMS.txt` from the same release.
3. Create a folder for the application, for example `C:\WinPanel`, and move the executable
   there. WinPanel does not need to live on the same drive as your servers.
4. Run `WinPanel.exe`.

On first start WinPanel:

- creates `%APPDATA%\WinPanel` with the database, log, backups and templates folders,
- writes a default `config.json`,
- starts the management interface on `http://127.0.0.1:8787`,
- opens it in Edge/Chrome as an app window (or your default browser).

Create the administrator account in the browser. The setup screen only exists while no
account has been created.

### Where the data lives

| Path | Contents |
| --- | --- |
| `config.json` | Bind address, remote access flag, log level, browser behaviour |
| `winpanel.db` | Users, sessions, servers, settings, backups, activity |
| `logs\winpanel.log` | Application log (rotated at 10 MB, previous kept as `.log.1`) |
| `servers\` | Default server directories |
| `backups\` | Backup archives per server |
| `templates\` | Optional user templates |
| `tmp\` | Install downloads and update staging |

Back up `winpanel.db` and `config.json` if you want to rebuild a machine. Server files live
in their own directories and can be copied independently.

---

## 2. Portable installation on another drive

```powershell
C:\Tools\WinPanel.exe --data-dir "D:\WinPanel"
```

Everything (database, servers, backups, logs) then lives under `D:\WinPanel`. Use this for
USB drives, secondary disks and machines with a small system partition. Keep the
executable's own path short — avoid spaces if you plan to script it.

To make the choice permanent without a command line, create a shortcut and add the
argument to its target:

```
C:\Tools\WinPanel.exe --data-dir "D:\WinPanel" --no-browser
```

---

## 3. Starting with Windows

### Task Scheduler (recommended)

1. Open **Task Scheduler** → **Create Task**.
2. General: name it `WinPanel`, select **Run whether user is logged on or not** if the
   machine is a dedicated host, and **Run with highest privileges** if your servers need it.
3. Triggers: **At startup**.
4. Actions: start `C:\WinPanel\WinPanel.exe` with arguments `--no-browser`.
5. Settings: enable **Restart the task if it fails** and disable **Stop the task if it runs
   longer than**.

WinPanel stops running servers gracefully on exit, but a hard kill (power loss, Task
Manager end-task) can leave server processes behind. Job Objects mark children to die with
the panel, so a crashed panel does not leak server processes.

### Startup folder

Copy a shortcut into `shell:startup`. This only starts the panel after sign-in and leaves a
console window visible, which is useful while you are learning the tool.

---

## 4. Opening the panel from another machine

The management API listens on loopback by default. To expose it:

1. Edit `%APPDATA%\WinPanel\config.json`:

   ```json
   {
     "listen": "0.0.0.0:8787",
     "allowRemote": true,
     "openBrowser": true,
     "logLevel": "info"
   }
   ```

2. Restart WinPanel. Both settings are required; otherwise it refuses to bind and explains
   why in the console and log.
3. Allow the port in Windows Firewall (run PowerShell **as administrator**):

   ```powershell
   New-NetFirewallRule -DisplayName "WinPanel" -Direction Inbound -Protocol TCP -LocalPort 8787 -Action Allow -Profile Private
   ```

4. Open `http://<machine-ip>:8787` from the other computer.

Notes:

- Traffic is HTTP. For internet access put a TLS reverse proxy in front of it, or only
  allow it on a trusted private network.
- Strong, unique passwords matter as soon as the panel leaves loopback.
- If you use a hostname, the browser sends an `Origin` header that must match the `Host`
  header; using the IP address avoids proxy rewrites breaking CSRF checks.

---

## 5. Building from source

Requirements:

- Rust stable 1.77+ (<https://rustup.rs>)
- Visual Studio Build Tools with the C++ workload (for the MSVC linker)
- Git

```powershell
git clone https://github.com/itriedcoding/WinPanel.git
cd WinPanel
cargo build --release
```

The executable is `target\release\winpanel.exe` (rename it to `WinPanel.exe` if you like).
SQLite is compiled into the binary; there is no external dependency to install.

Development loop:

```powershell
cargo run -- --data-dir .\devdata
```

In debug builds the web assets are read from `web/`, so refreshing the browser picks up
front-end changes. Release builds embed them.

---

## 6. Uninstalling

1. Stop WinPanel (Ctrl+C in its console, or end the process).
2. Delete the executable.
3. Delete `%APPDATA%\WinPanel` (or your custom data directory) to remove the database,
   server files, backups and logs.

Uninstalling does not remove Windows Firewall rules you created manually.
