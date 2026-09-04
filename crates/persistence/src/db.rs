//! The SQLite connection: open sequence, pragmas, integrity check and the
//! move-aside recovery for a damaged file.
//!
//! One connection per process behind a mutex (`rusqlite::Connection` is `Send`
//! but not `Sync`). Callers run every access on a blocking thread; the core
//! never touches the database from a command handler or the event loop.
//!
//! Open sequence (ADR-014): create the parent directory → open → `journal_mode
//! = WAL` (outside any transaction), `synchronous = NORMAL`, `foreign_keys = ON`,
//! `busy_timeout` → migrate. A file that is not a database or is corrupt is
//! renamed to `<name>.corrupt-<unix-ms>` (with its `-wal` / `-shm` siblings)
//! and a fresh database takes its place; the caller shows the notice. If the
//! fresh database fails as well the error is returned: the caller runs
//! without persistence, never crashes and never loops.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Transaction};

use crate::error::PersistenceError;
use crate::migrations::{self, MIGRATIONS, MigrationReport};

/// File name inside the application data directory.
pub const DB_FILE_NAME: &str = "gritty.sqlite";

/// How long a statement waits for a lock held by another connection.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Database {
    conn: Mutex<Connection>,
    path: PathBuf,
    /// Test hook: how many upcoming `write` calls fail before touching the
    /// connection (see [`Database::inject_write_failures`]).
    write_faults: AtomicU32,
}

/// What `Database::open` did.
#[derive(Debug)]
pub struct Opened {
    pub database: Database,
    /// Where a damaged file was moved before a fresh database was created.
    pub moved_aside: Option<PathBuf>,
    pub migrations: MigrationReport,
    pub elapsed: Duration,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Open (creating if needed) and migrate the database at `path`.
    pub fn open(path: &Path) -> Result<Opened, PersistenceError> {
        let started = Instant::now();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                PersistenceError::io(format!("creating {}", parent.display()), error)
            })?;
        }
        let (conn, migrations, moved_aside) = match open_prepared(path) {
            Ok((conn, report)) => (conn, report, None),
            Err(error) if error.is_corrupt() => {
                let aside = move_aside(path)?;
                tracing::warn!(
                    path = %path.display(),
                    moved_to = %aside.display(),
                    %error,
                    "database is unusable; moved aside and starting fresh"
                );
                let (conn, report) = open_prepared(path)?;
                (conn, report, Some(aside))
            }
            Err(error) => return Err(error),
        };
        let elapsed = started.elapsed();
        tracing::info!(
            path = %path.display(),
            from = migrations.from,
            to = migrations.to,
            elapsed_ms = elapsed.as_millis() as u64,
            "database open"
        );
        Ok(Opened {
            database: Self {
                conn: Mutex::new(conn),
                path: path.to_path_buf(),
                write_faults: AtomicU32::new(0),
            },
            moved_aside,
            migrations,
            elapsed,
        })
    }

    /// A migrated in-memory database (tests, and the placeholder used while a
    /// damaged file is being replaced).
    pub fn open_in_memory() -> Result<Self, PersistenceError> {
        let mut conn = Connection::open_in_memory()?;
        migrations::migrate(&mut conn, MIGRATIONS, now_ms())?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
            write_faults: AtomicU32::new(0),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a read against the connection.
    pub fn read<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let conn = self.lock();
        work(&conn)
    }

    /// Make the next `count` calls to [`Self::write`] fail as if the disk
    /// refused them (test support for retry and backoff paths; production
    /// code never calls this).
    pub fn inject_write_failures(&self, count: u32) {
        self.write_faults.store(count, Ordering::SeqCst);
    }

    /// Run `work` inside one transaction; an error rolls everything back.
    pub fn write<T>(
        &self,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        if self
            .write_faults
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(PersistenceError::io(
                "injected write failure",
                std::io::Error::other("disk refused the write"),
            ));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// The active journal mode (`wal` for a file database).
    pub fn journal_mode(&self) -> Result<String, PersistenceError> {
        let mode: String = self
            .lock()
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        Ok(mode)
    }

    /// `PRAGMA quick_check`: `Ok` when the file is consistent, `Corrupt` otherwise.
    /// Costs a pass over the file; run it off the startup path.
    pub fn quick_check(&self) -> Result<(), PersistenceError> {
        let conn = self.lock();
        let mut statement = conn.prepare("PRAGMA quick_check")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut problems = Vec::new();
        for row in rows {
            let line = row?;
            if line != "ok" {
                problems.push(line);
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(PersistenceError::Corrupt {
                path: self.path.clone(),
                reason: problems.join("; "),
            })
        }
    }

    /// Replace a damaged database while running: close it, move the file
    /// aside, open a fresh migrated one under the same path. Returns where the
    /// old file went. On failure the handle holds an empty in-memory database
    /// and the caller should stop persisting.
    pub fn replace_corrupt(&self) -> Result<(PathBuf, MigrationReport), PersistenceError> {
        let mut guard = self.lock();
        let placeholder = Connection::open_in_memory()?;
        let old = std::mem::replace(&mut *guard, placeholder);
        if let Err((_, error)) = old.close() {
            tracing::warn!(%error, "closing the damaged database reported an error");
        }
        let aside = move_aside(&self.path)?;
        let (conn, report) = open_prepared(&self.path)?;
        *guard = conn;
        tracing::warn!(
            path = %self.path.display(),
            moved_to = %aside.display(),
            "database replaced after a failed integrity check"
        );
        Ok((aside, report))
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn open_prepared(path: &Path) -> Result<(Connection, MigrationReport), PersistenceError> {
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    let report = migrations::migrate(&mut conn, MIGRATIONS, now_ms())?;
    Ok((conn, report))
}

fn configure(conn: &Connection) -> Result<(), PersistenceError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !matches!(mode.as_str(), "wal" | "memory") {
        tracing::warn!(
            mode,
            "WAL journal mode not available; continuing with the reported mode"
        );
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Rename the database and its WAL/shm siblings so a fresh file can be created.
fn move_aside(path: &Path) -> Result<PathBuf, PersistenceError> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| DB_FILE_NAME.to_owned());
    let target = path.with_file_name(format!("{name}.corrupt-{}", now_ms()));
    fs::rename(path, &target).map_err(|error| {
        PersistenceError::io(
            format!("moving {} to {}", path.display(), target.display()),
            error,
        )
    })?;
    for suffix in ["-wal", "-shm"] {
        let sibling = path.with_file_name(format!("{name}{suffix}"));
        if sibling.exists() {
            let sibling_target = target.with_file_name(format!(
                "{}{suffix}",
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
            if let Err(error) = fs::rename(&sibling, &sibling_target) {
                tracing::warn!(path = %sibling.display(), %error, "could not move a journal sibling aside");
            }
        }
    }
    Ok(target)
}

/// Milliseconds since the Unix epoch, saturating.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn move_aside_renames_the_file_and_its_journal_siblings() {
        let dir = TempDir::new("move-aside");
        let path = dir.join(DB_FILE_NAME);
        fs::write(&path, b"main").unwrap();
        fs::write(dir.join("gritty.sqlite-wal"), b"wal").unwrap();
        fs::write(dir.join("gritty.sqlite-shm"), b"shm").unwrap();
        fs::write(dir.join("unrelated.txt"), b"keep").unwrap();

        let aside = move_aside(&path).unwrap();
        assert!(!path.exists());
        let name = aside.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("gritty.sqlite.corrupt-"), "{name}");
        assert_eq!(fs::read(&aside).unwrap(), b"main");
        assert_eq!(fs::read(dir.join(format!("{name}-wal"))).unwrap(), b"wal");
        assert_eq!(fs::read(dir.join(format!("{name}-shm"))).unwrap(), b"shm");
        assert!(!dir.join("gritty.sqlite-wal").exists());
        assert!(!dir.join("gritty.sqlite-shm").exists());
        assert!(dir.join("unrelated.txt").exists());
    }

    #[test]
    fn move_aside_of_a_missing_file_is_an_io_error() {
        let dir = TempDir::new("move-aside-missing");
        let error = move_aside(&dir.join(DB_FILE_NAME)).unwrap_err();
        assert!(matches!(error, PersistenceError::Io { .. }), "{error}");
        assert!(!error.is_corrupt());
    }
}
