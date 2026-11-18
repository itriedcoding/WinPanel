# Server templates

A template describes *how* an application is installed, started and stopped. Templates are
JSON data: adding a new server type never requires changing WinPanel's code.

## Where templates live

- Built-in templates are compiled into the executable from `templates/builtin.json`.
- User templates are loaded from `<data>/templates/*.json` at startup and whenever
  **Templates → Reload templates** is pressed.
- A user template with the same `id` as a built-in replaces it.
- Invalid templates are skipped and reported in the Templates screen instead of breaking
  the panel.

## Schema

```json
{
  "schemaVersion": 1,
  "templates": [
    {
      "id": "my-app",
      "name": "My application",
      "category": "Applications",
      "vendor": "Example",
      "description": "What this template installs.",
      "install": {
        "method": "download",
        "url": "https://example.com/releases/latest/app.zip",
        "archive": "zip",
        "target": "app.zip"
      },
      "runtime": [{ "kind": "java", "minimumMajor": 17, "required": true }],
      "startupExe": "java",
      "startupArgs": "-Xmx{{memoryMb}}M -jar app.jar",
      "stopCommand": "stop",
      "port": 8080,
      "protocol": "tcp",
      "env": [{ "key": "APP_ENV", "value": "production" }],
      "memoryMb": 2048,
      "storageMb": 10240,
      "options": [
        {
          "id": "version",
          "label": "Version",
          "kind": "text",
          "default": "latest",
          "required": true,
          "help": "Which release to install."
        }
      ],
      "files": [
        {
          "path": "config/app.yml",
          "content": "port: {{port}}\nworld: {{worldName}}\n"
        }
      ],
      "notes": "Shown in the create dialog."
    }
  ]
}
```

A file may also contain a bare array of template objects.

### Fields

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Required. Lowercase letters, digits, `-`, `_`. Unique. |
| `name` | string | Required display name. |
| `category` | string | Grouping in the Templates screen. |
| `vendor` | string | Small metadata line. |
| `description` | string | One or two sentences. |
| `install.method` | string | `none`, `download`, `minecraft_vanilla`, `paper`, `bedrock`, `fivem`. |
| `install.url` | string | Required for `download`. Supports `{{placeholders}}`. |
| `install.archive` | string | `none` keeps the download as a file, `zip` extracts it into the server directory. |
| `install.target` | string | Filename for the download (e.g. `server.jar`). |
| `runtime` | array | Required runtimes: `java`, `node`, `python`. |
| `runtime[].minimumMajor` | number | Minimum major version, verified by running the runtime's `--version`. |
| `runtime[].required` | bool | Defaults to `true`. |
| `startupExe` | string | Executable name, relative file or absolute path. |
| `startupArgs` | string | Quote-aware arguments. No shell. |
| `stopCommand` | string | Sent to stdin before terminating. |
| `port` / `protocol` | number / string | Default port and `tcp`/`udp`. |
| `env` | array | Default environment variables. |
| `memoryMb` / `storageMb` | number | Defaults offered in the create dialog. |
| `options` | array | Inputs shown when creating the server. |
| `files` | array | Scaffolded files, written after installation. |
| `notes` | string | Shown as a note in the create dialog and details modal. |

### Option types

`kind` is one of `text`, `number`, `bool`, `select`, `password`.

```json
{
  "id": "acceptEula",
  "label": "Accept the EULA",
  "kind": "bool",
  "default": "true",
  "help": "Required by the vendor."
}
```

For `select`, provide `choices`:

```json
{
  "id": "channel",
  "label": "Channel",
  "kind": "select",
  "default": "stable",
  "choices": [
    { "value": "stable", "label": "Stable" },
    { "value": "beta", "label": "Beta" }
  ]
}
```

### Placeholders

`{{name}}` tokens are replaced when files are written, when the startup command is prepared
and inside `install.url`/`install.target`:

| Placeholder | Value |
| --- | --- |
| `{{port}}` | First configured port |
| `{{memoryMb}}` | Configured memory limit, minimum 256 |
| `{{serverName}}` | Server name (or an option with the same id) |
| any option id | The value the user entered |

Option values are strings (`"true"`/`"false"` for booleans), so `eula={{acceptEula}}` writes
`eula=true`. Unknown placeholders are left untouched rather than silently dropped.

## Install methods

| Method | Behaviour |
| --- | --- |
| `none` | No download. Runtime checks and file scaffolding still run. |
| `download` | Streams `install.url` and optionally extracts a zip. |
| `minecraft_vanilla` | Reads Mojang's version manifest, resolves the release (or the `mcVersion` option) and downloads `server.jar`. |
| `paper` | Uses the PaperMC Fill v3 API to find the newest build for the requested (or latest stable) Minecraft version, then verifies the SHA-256 checksum Paper publishes. |
| `bedrock` | Resolves the Bedrock Dedicated Server download for the current platform from Microsoft's service, with a documented direct-download fallback. |
| `fivem` | Downloads the recommended Windows FXServer artifact and writes `run.cmd` if the archive does not contain one. |

Installation steps always:

1. validate required runtimes and their versions,
2. download and extract into the server directory,
3. write scaffolded `files` with placeholders resolved,
4. store the resolved startup command and options on the server.

Failures are recorded on the server (`installError`) and in the activity log with the
upstream error message.

## Runtime detection

WinPanel searches `PATH` (`PATHEXT` aware on Windows) and common installation directories:
`JAVA_HOME`, `%ProgramFiles%\Java`, Eclipse Adoptium, Microsoft JDK, `nodejs`,
`%LOCALAPPDATA%\Programs\Python`, Chocolatey's bin folder, `/usr/local/bin` and `/usr/bin` on
Unix. Versions are read by executing the runtime with `-version`/`--version` and a short
timeout.

## Safety rules enforced on load

- `id` must be lowercase ASCII with `-`/`_`.
- `install.method` must be known.
- `download` requires a URL.
- Scaffolded file paths must be relative and must not contain `..`.

## Example: Fabric server

Fabric publishes a ready server launcher per loader/game combination. Because the URL
depends on two versions, the simplest robust template downloads a fixed URL and lets the
user update it:

```json
{
  "templates": [
    {
      "id": "fabric",
      "name": "Minecraft: Fabric",
      "category": "Minecraft",
      "description": "Fabric server loader. Install the launcher jar, then start it with Java.",
      "install": {
        "method": "download",
        "url": "https://meta.fabricmc.net/v2/versions/loader/1.21.4/0.16.9/1.0.1/server/jar",
        "archive": "none",
        "target": "server.jar"
      },
      "runtime": [{ "kind": "java", "minimumMajor": 21 }],
      "startupExe": "java",
      "startupArgs": "-Xms512M -Xmx{{memoryMb}}M -jar server.jar nogui",
      "stopCommand": "stop",
      "port": 25565,
      "memoryMb": 4096,
      "storageMb": 20480,
      "files": [
        { "path": "eula.txt", "content": "eula=true\n" },
        { "path": "server.properties", "content": "server-port={{port}}\n" }
      ],
      "notes": "Update the Fabric loader URL when you want a newer loader."
    }
  ]
}
```

Save it as `%APPDATA%\WinPanel\templates\fabric.json`, press **Reload templates**, and the
template appears in the create dialog.
