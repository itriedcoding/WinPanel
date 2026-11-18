# Security

WinPanel runs with the privileges of the account that starts it, and every server it
manages inherits those privileges. Treat the panel as an administrative tool: anyone who
can sign in can run arbitrary commands inside server directories.

## Threat model

| Threat | Mitigation |
| --- | --- |
| Network exposure by accident | Loopback bind is the default. A non-loopback address is only accepted with an explicit `allowRemote` setting or `--allow-remote`. |
| Password theft from the database | Argon2id with per-password salts. Plaintext passwords never touch the database, logs or API responses. |
| Session hijacking | 256-bit random session tokens, stored as SHA-256 hashes; `HttpOnly`, `SameSite=Strict`, `Path=/` cookies. Tokens rotate on sign-in and password change. |
| Cross-site request forgery | Per-session CSRF token required on every mutation, plus `Origin` vs `Host` validation. |
| Brute force | Five failed sign-ins per username lock further attempts for five minutes. |
| Privilege escalation | Admin-only endpoints check the role; every server endpoint re-checks the per-server access level on the server side. The last active administrator cannot be demoted, disabled or deleted. |
| Path traversal | Client paths are normalised (no `..`, absolute paths, drive letters or NUL bytes), then canonicalised and required to remain under the server root. Symlink escapes are rejected. |
| Command injection | No shell is used. Startup commands are split into an argument vector and executed directly; console input is written to the child's stdin as data. |
| Process leaks | Servers run in a Windows Job Object with `KILL_ON_JOB_CLOSE`, so the tree dies with the panel or when it is stopped. |
| Arbitrary file read through uploads | Uploads are written to a temporary sibling file, size-limited and quota-checked before being renamed into place. |
| Zip-slip during backup restore | `zip::ZipArchive::enclosed_name` is used for every entry; the archive is extracted into a staging directory that is swapped in only after a successful extraction. |
| Update tampering | The release asset's SHA-256 digest and byte size are verified before the executable is swapped, and Windows binaries are checked for the `MZ` header. |
| Sensitive data in the UI | No tokens are placed in `localStorage`; only console command history is kept there. Cookies are never exposed to JavaScript. |

## Operational guidance

- Keep the panel on loopback unless you genuinely need remote access.
- If remote access is required, restrict the firewall rule to a private profile or VPN,
  terminate TLS at a reverse proxy, and use long unique passwords.
- Run the panel from a dedicated Windows account when hosting untrusted servers, so a
  compromised game server cannot read unrelated user files.
- Server processes run inside job objects, but they are not OS sandboxes: a malicious
  executable downloaded by a server plugin still runs as your user.
- Review **Settings → Activity log** after changes to accounts or permissions.
- Back up `winpanel.db` and `config.json` before upgrades.

## Reporting a vulnerability

Please do not open a public issue for security problems. Send a private report through
GitHub's **Security → Report a vulnerability** flow on
<https://github.com/itriedcoding/WinPanel> with:

- affected version (Settings → About),
- reproduction steps,
- impact assessment,
- any suggested fix.

You will get an acknowledgement, and a fix will be published in a normal release with credit
if you want it.

## Known limitations

- The management interface speaks HTTP. TLS is the responsibility of a reverse proxy when
  exposed beyond a trusted network.
- There is no two-factor authentication or API token system; sessions only.
- Memory and CPU limits are enforced by Windows Job Objects. On Linux/macOS the panel logs
  that limits are Windows-only and relies on affinity where available.
- Server processes are not filesystem-jailed; file manager sandboxing applies to the panel's
  own API, not to the child process itself.
