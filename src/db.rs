//! SQLite persistence layer.
//!
//! All access goes through a single mutex-guarded connection running in WAL
//! mode: writes are atomic, and the connection is never shared across threads
//! without the lock. Corrupt rows are reported, never silently replaced.

use crate::error::Result;
use crate::models::*;
use crate::util::now_ms;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const SCHEMA_VERSION: i32 = 1;

fn json_vec<T: serde::de::DeserializeOwned>(raw: &str) -> Vec<T> {
    serde_json::from_str(raw).unwrap_or_default()
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.lock();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        conn.execute_batch(
            r#"
            BEGIN;
            CREATE TABLE IF NOT EXISTS users (
                id              TEXT PRIMARY KEY,
                username        TEXT NOT NULL UNIQUE COLLATE NOCASE,
                password_hash   TEXT NOT NULL,
                role            TEXT NOT NULL DEFAULT 'user',
                created_at      INTEGER NOT NULL,
                last_login      INTEGER,
                disabled        INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                csrf        TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                expires_at  INTEGER NOT NULL,
                ip          TEXT,
                user_agent  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

            CREATE TABLE IF NOT EXISTS servers (
                id                      TEXT PRIMARY KEY,
                name                    TEXT NOT NULL,
                description             TEXT NOT NULL DEFAULT '',
                template_id             TEXT NOT NULL DEFAULT '',
                install_dir             TEXT NOT NULL,
                startup_exe             TEXT NOT NULL,
                startup_args            TEXT NOT NULL DEFAULT '',
                env                     TEXT NOT NULL DEFAULT '[]',
                memory_mb               INTEGER NOT NULL DEFAULT 0,
                cpu_cores               TEXT NOT NULL DEFAULT '[]',
                cpu_limit_percent       INTEGER NOT NULL DEFAULT 0,
                storage_mb              INTEGER NOT NULL DEFAULT 0,
                ports                   TEXT NOT NULL DEFAULT '[]',
                auto_restart            INTEGER NOT NULL DEFAULT 0,
                auto_restart_max        INTEGER NOT NULL DEFAULT 3,
                auto_restart_delay_ms   INTEGER NOT NULL DEFAULT 5000,
                stop_command            TEXT NOT NULL DEFAULT 'stop',
                stop_timeout_ms         INTEGER NOT NULL DEFAULT 30000,
                status                  TEXT NOT NULL DEFAULT 'stopped',
                installed               INTEGER NOT NULL DEFAULT 0,
                install_error           TEXT,
                created_at              INTEGER NOT NULL,
                updated_at              INTEGER NOT NULL,
                created_by              TEXT,
                last_exit_code          INTEGER,
                backups_enabled         INTEGER NOT NULL DEFAULT 0,
                backup_interval_hours   INTEGER NOT NULL DEFAULT 24,
                backup_keep             INTEGER NOT NULL DEFAULT 5,
                install_options         TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS server_access (
                user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
                level       TEXT NOT NULL DEFAULT 'view',
                PRIMARY KEY (user_id, server_id)
            );

            CREATE TABLE IF NOT EXISTS activity (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                ts          INTEGER NOT NULL,
                kind        TEXT NOT NULL,
                server_id   TEXT,
                user_id     TEXT,
                message     TEXT NOT NULL,
                detail      TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_activity_ts ON activity(ts DESC);

            CREATE TABLE IF NOT EXISTS backups (
                id          TEXT PRIMARY KEY,
                server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
                name        TEXT NOT NULL,
                path        TEXT NOT NULL,
                size        INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL,
                status      TEXT NOT NULL DEFAULT 'complete',
                checksum    TEXT,
                error       TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_backups_server ON backups(server_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS settings (
                key     TEXT PRIMARY KEY,
                value   TEXT NOT NULL
            );
            COMMIT;
            "#,
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    // ---------------------------------------------------------------- users

    pub fn count_users(&self) -> Result<i64> {
        let conn = self.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?)
    }

    pub fn create_user(&self, id: &str, username: &str, hash: &str, role: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, role, created_at, disabled)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![id, username, hash, role, now_ms()],
        )?;
        Ok(())
    }

    pub fn find_credential(&self, username: &str) -> Result<Option<UserCredential>> {
        let conn = self.lock();
        let result = conn
            .query_row(
                "SELECT id, username, role, created_at, last_login, disabled, password_hash
                 FROM users WHERE username = ?1 COLLATE NOCASE",
                params![username],
                |row| {
                    Ok(UserCredential {
                        user: User {
                            id: row.get(0)?,
                            username: row.get(1)?,
                            role: row.get(2)?,
                            created_at: row.get(3)?,
                            last_login: row.get(4)?,
                            disabled: row.get::<_, i64>(5)? != 0,
                        },
                        password_hash: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    pub fn user_by_id(&self, id: &str) -> Result<Option<User>> {
        let conn = self.lock();
        let user = conn
            .query_row(
                "SELECT id, username, role, created_at, last_login, disabled
                 FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        role: row.get(2)?,
                        created_at: row.get(3)?,
                        last_login: row.get(4)?,
                        disabled: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?;
        Ok(user)
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, role, created_at, last_login, disabled FROM users ORDER BY username",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
                last_login: row.get(4)?,
                disabled: row.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn update_password(&self, user_id: &str, hash: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET password_hash = ?2 WHERE id = ?1",
            params![user_id, hash],
        )?;
        Ok(())
    }

    pub fn update_role(&self, user_id: &str, role: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET role = ?2 WHERE id = ?1",
            params![user_id, role],
        )?;
        Ok(())
    }

    pub fn set_disabled(&self, user_id: &str, disabled: bool) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET disabled = ?2 WHERE id = ?1",
            params![user_id, if disabled { 1 } else { 0 }],
        )?;
        Ok(())
    }

    pub fn delete_user(&self, user_id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        Ok(())
    }

    pub fn touch_login(&self, user_id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET last_login = ?2 WHERE id = ?1",
            params![user_id, now_ms()],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------- sessions

    pub fn create_session(
        &self,
        id: &str,
        user_id: &str,
        csrf: &str,
        expires_at: i64,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO sessions (id, user_id, csrf, created_at, expires_at, ip, user_agent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, user_id, csrf, now_ms(), expires_at, ip, user_agent],
        )?;
        Ok(())
    }

    pub fn session(&self, id: &str) -> Result<Option<Session>> {
        let conn = self.lock();
        let session = conn
            .query_row(
                "SELECT user_id, csrf, created_at, expires_at FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Session {
                        user_id: row.get(0)?,
                        csrf: row.get(1)?,
                        created_at: row.get(2)?,
                        expires_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_user_sessions(&self, user_id: &str, except: Option<&str>) -> Result<()> {
        let conn = self.lock();
        match except {
            Some(keep) => conn.execute(
                "DELETE FROM sessions WHERE user_id = ?1 AND id <> ?2",
                params![user_id, keep],
            )?,
            None => conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?,
        };
        Ok(())
    }

    pub fn purge_expired_sessions(&self) -> Result<usize> {
        let conn = self.lock();
        Ok(conn.execute(
            "DELETE FROM sessions WHERE expires_at < ?1",
            params![now_ms()],
        )?)
    }

    // -------------------------------------------------------------- servers

    pub fn insert_server(&self, server: &Server) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO servers (
                id, name, description, template_id, install_dir, startup_exe, startup_args,
                env, memory_mb, cpu_cores, cpu_limit_percent, storage_mb, ports, auto_restart,
                auto_restart_max, auto_restart_delay_ms, stop_command, stop_timeout_ms, status,
                installed, install_error, created_at, updated_at, created_by, last_exit_code,
                backups_enabled, backup_interval_hours, backup_keep, install_options
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                       ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)",
            params![
                server.id,
                server.name,
                server.description,
                server.template_id,
                server.install_dir,
                server.startup_exe,
                server.startup_args,
                serde_json::to_string(&server.env)?,
                server.memory_mb as i64,
                serde_json::to_string(&server.cpu_cores)?,
                server.cpu_limit_percent as i64,
                server.storage_mb as i64,
                serde_json::to_string(&server.ports)?,
                server.auto_restart as i64,
                server.auto_restart_max as i64,
                server.auto_restart_delay_ms as i64,
                server.stop_command,
                server.stop_timeout_ms as i64,
                server.status,
                server.installed as i64,
                server.install_error,
                server.created_at,
                server.updated_at,
                server.created_by,
                server.last_exit_code,
                server.backups_enabled as i64,
                server.backup_interval_hours as i64,
                server.backup_keep as i64,
                serde_json::to_string(&server.install_options)?,
            ],
        )?;
        Ok(())
    }

    pub fn update_server(&self, server: &Server) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE servers SET
                name = ?2, description = ?3, template_id = ?4, install_dir = ?5,
                startup_exe = ?6, startup_args = ?7, env = ?8, memory_mb = ?9,
                cpu_cores = ?10, cpu_limit_percent = ?11, storage_mb = ?12, ports = ?13,
                auto_restart = ?14, auto_restart_max = ?15, auto_restart_delay_ms = ?16,
                stop_command = ?17, stop_timeout_ms = ?18, status = ?19, installed = ?20,
                install_error = ?21, updated_at = ?22, last_exit_code = ?23,
                backups_enabled = ?24, backup_interval_hours = ?25, backup_keep = ?26,
                install_options = ?27
             WHERE id = ?1",
            params![
                server.id,
                server.name,
                server.description,
                server.template_id,
                server.install_dir,
                server.startup_exe,
                server.startup_args,
                serde_json::to_string(&server.env)?,
                server.memory_mb as i64,
                serde_json::to_string(&server.cpu_cores)?,
                server.cpu_limit_percent as i64,
                server.storage_mb as i64,
                serde_json::to_string(&server.ports)?,
                server.auto_restart as i64,
                server.auto_restart_max as i64,
                server.auto_restart_delay_ms as i64,
                server.stop_command,
                server.stop_timeout_ms as i64,
                server.status,
                server.installed as i64,
                server.install_error,
                server.updated_at,
                server.last_exit_code,
                server.backups_enabled as i64,
                server.backup_interval_hours as i64,
                server.backup_keep as i64,
                serde_json::to_string(&server.install_options)?,
            ],
        )?;
        Ok(())
    }

    pub fn set_server_status(
        &self,
        id: &str,
        status: &str,
        last_exit_code: Option<i32>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE servers SET status = ?2, last_exit_code = COALESCE(?3, last_exit_code), updated_at = ?4
             WHERE id = ?1",
            params![id, status, last_exit_code, now_ms()],
        )?;
        Ok(())
    }

    pub fn set_install_state(
        &self,
        id: &str,
        installed: bool,
        error: Option<&str>,
        status: &str,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE servers SET installed = ?2, install_error = ?3, status = ?4, updated_at = ?5
             WHERE id = ?1",
            params![id, installed as i64, error, status, now_ms()],
        )?;
        Ok(())
    }

    pub fn rename_server(&self, id: &str, name: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE servers SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, now_ms()],
        )?;
        Ok(())
    }

    pub fn delete_server(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM servers WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn server_by_id(&self, id: &str) -> Result<Option<Server>> {
        let conn = self.lock();
        let server = conn
            .query_row(
                "SELECT * FROM servers WHERE id = ?1",
                params![id],
                row_to_server,
            )
            .optional()?;
        Ok(server)
    }

    pub fn list_servers(&self) -> Result<Vec<Server>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT * FROM servers ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], row_to_server)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Servers visible to a user, with the access level. Admins see everything.
    pub fn list_servers_visible(&self, user: &User) -> Result<Vec<(Server, String)>> {
        let all = self.list_servers()?;
        if user.role == ROLE_ADMIN {
            return Ok(all.into_iter().map(|s| (s, "manage".to_string())).collect());
        }
        let access = self.list_access(user.id.as_str())?;
        Ok(all
            .into_iter()
            .filter_map(|server| {
                let level = access
                    .iter()
                    .find(|a| a.server_id == server.id)
                    .map(|a| a.level.clone());
                level.map(|l| (server, l))
            })
            .collect())
    }

    // --------------------------------------------------------------- access

    pub fn set_access(&self, user_id: &str, server_id: &str, level: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO server_access (user_id, server_id, level) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id, server_id) DO UPDATE SET level = excluded.level",
            params![user_id, server_id, level],
        )?;
        Ok(())
    }

    pub fn remove_access(&self, user_id: &str, server_id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM server_access WHERE user_id = ?1 AND server_id = ?2",
            params![user_id, server_id],
        )?;
        Ok(())
    }

    pub fn list_access(&self, user_id: &str) -> Result<Vec<ServerAccess>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT server_id, level FROM server_access WHERE user_id = ?1")?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok(ServerAccess {
                server_id: row.get(0)?,
                level: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn list_server_users(&self, server_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT user_id, level FROM server_access WHERE server_id = ?1")?;
        let rows = stmt.query_map(params![server_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn access_level(&self, user_id: &str, server_id: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let level = conn
            .query_row(
                "SELECT level FROM server_access WHERE user_id = ?1 AND server_id = ?2",
                params![user_id, server_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(level)
    }

    // ------------------------------------------------------------- activity

    pub fn log_activity(
        &self,
        kind: &str,
        server_id: Option<&str>,
        user_id: Option<&str>,
        message: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO activity (ts, kind, server_id, user_id, message, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now_ms(), kind, server_id, user_id, message, detail],
        )?;
        Ok(())
    }

    pub fn recent_activity(
        &self,
        limit: i64,
        server_id: Option<&str>,
    ) -> Result<Vec<ActivityEntry>> {
        let conn = self.lock();
        let mut sql = String::from(
            "SELECT a.id, a.ts, a.kind, a.server_id, s.name, a.user_id, u.username,
                    a.message, a.detail
             FROM activity a
             LEFT JOIN servers s ON s.id = a.server_id
             LEFT JOIN users u ON u.id = a.user_id",
        );
        if server_id.is_some() {
            sql.push_str(" WHERE a.server_id = ?2");
        }
        sql.push_str(" ORDER BY a.id DESC LIMIT ?1");
        let mut stmt = conn.prepare(&sql)?;
        let map = |row: &Row<'_>| -> rusqlite::Result<ActivityEntry> {
            Ok(ActivityEntry {
                id: row.get(0)?,
                ts: row.get(1)?,
                kind: row.get(2)?,
                server_id: row.get(3)?,
                server_name: row.get(4)?,
                user_id: row.get(5)?,
                username: row.get(6)?,
                message: row.get(7)?,
                detail: row.get(8)?,
            })
        };
        let rows = match server_id {
            Some(id) => stmt.query_map(params![limit, id], map)?,
            None => stmt.query_map(params![limit], map)?,
        };
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn prune_activity(&self, keep: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM activity WHERE id NOT IN (SELECT id FROM activity ORDER BY id DESC LIMIT ?1)",
            params![keep],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------- backups

    pub fn insert_backup(&self, backup: &Backup, path: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO backups (id, server_id, name, path, size, created_at, status, checksum, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                backup.id,
                backup.server_id,
                backup.name,
                path,
                backup.size as i64,
                backup.created_at,
                backup.status,
                backup.checksum,
                backup.error
            ],
        )?;
        Ok(())
    }

    pub fn update_backup(
        &self,
        id: &str,
        size: u64,
        status: &str,
        checksum: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE backups SET size = ?2, status = ?3, checksum = ?4, error = ?5 WHERE id = ?1",
            params![id, size as i64, status, checksum, error],
        )?;
        Ok(())
    }

    pub fn list_backups(&self, server_id: &str) -> Result<Vec<Backup>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, server_id, name, size, created_at, status, checksum, error
             FROM backups WHERE server_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![server_id], |row| {
            Ok(Backup {
                id: row.get(0)?,
                server_id: row.get(1)?,
                name: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                created_at: row.get(4)?,
                status: row.get(5)?,
                checksum: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn backup_by_id(&self, id: &str) -> Result<Option<(Backup, String)>> {
        let conn = self.lock();
        let found = conn
            .query_row(
                "SELECT id, server_id, name, size, created_at, status, checksum, error, path
                 FROM backups WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        Backup {
                            id: row.get(0)?,
                            server_id: row.get(1)?,
                            name: row.get(2)?,
                            size: row.get::<_, i64>(3)? as u64,
                            created_at: row.get(4)?,
                            status: row.get(5)?,
                            checksum: row.get(6)?,
                            error: row.get(7)?,
                        },
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()?;
        Ok(found)
    }

    pub fn delete_backup(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM backups WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn all_backups(&self) -> Result<Vec<Backup>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, server_id, name, size, created_at, status, checksum, error
             FROM backups ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Backup {
                id: row.get(0)?,
                server_id: row.get(1)?,
                name: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                created_at: row.get(4)?,
                status: row.get(5)?,
                checksum: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    // ------------------------------------------------------------- settings

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let value = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn row_to_server(row: &Row<'_>) -> rusqlite::Result<Server> {
    let env: String = row.get("env")?;
    let cpu_cores: String = row.get("cpu_cores")?;
    let ports: String = row.get("ports")?;
    Ok(Server {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        template_id: row.get("template_id")?,
        install_dir: row.get("install_dir")?,
        startup_exe: row.get("startup_exe")?,
        startup_args: row.get("startup_args")?,
        env: json_vec(&env),
        memory_mb: row.get::<_, i64>("memory_mb")? as u64,
        cpu_cores: json_vec(&cpu_cores),
        cpu_limit_percent: row.get::<_, i64>("cpu_limit_percent")? as u64,
        storage_mb: row.get::<_, i64>("storage_mb")? as u64,
        ports: json_vec(&ports),
        auto_restart: row.get::<_, i64>("auto_restart")? != 0,
        auto_restart_max: row.get::<_, i64>("auto_restart_max")? as u32,
        auto_restart_delay_ms: row.get::<_, i64>("auto_restart_delay_ms")? as u64,
        stop_command: row.get("stop_command")?,
        stop_timeout_ms: row.get::<_, i64>("stop_timeout_ms")? as u64,
        status: row.get("status")?,
        installed: row.get::<_, i64>("installed")? != 0,
        install_error: row.get("install_error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        created_by: row.get("created_by")?,
        last_exit_code: row.get("last_exit_code")?,
        backups_enabled: row.get::<_, i64>("backups_enabled")? != 0,
        backup_interval_hours: row.get::<_, i64>("backup_interval_hours")? as u32,
        backup_keep: row.get::<_, i64>("backup_keep")? as u32,
        install_options: serde_json::from_str::<std::collections::BTreeMap<String, String>>(
            &row.get::<_, String>("install_options")?,
        )
        .unwrap_or_default(),
    })
}
