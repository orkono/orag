//! Ordered, transactional, embedded schema migrations (D-017).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{OragError, Result};

pub const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_initial.sql"),
    include_str!("migrations/0002_meta.sql"),
];
pub const SUPPORTED_SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub fn migrate(conn: &mut Connection, db_path: &Path) -> Result<u32> {
    migrate_with(conn, db_path, MIGRATIONS)
}

/// Applies `migrations[user_version..]`, one transaction each, and refuses a
/// newer schema. A current database is checked without taking the write
/// lock, so opening never waits for another process's writes.
///
/// Migrations run with foreign keys off, so a table rebuild (create, copy,
/// drop, rename) never fires `ON DELETE CASCADE`; `foreign_key_check` must
/// pass before each step commits. Each step takes the write lock first
/// (`BEGIN IMMEDIATE`, waiting up to `MIGRATION_WAIT` for another process's
/// migration) and re-reads the version inside it, so processes opening the
/// same database at once apply every migration exactly once and still refuse
/// a schema another process raised meanwhile. An existing database (version
/// above 0 when this call starts) gets its own complete backup first.
pub fn migrate_with(conn: &mut Connection, db_path: &Path, migrations: &[&str]) -> Result<u32> {
    let target = migrations.len() as u32;
    let current = schema_version(conn)?;
    if current > target {
        return Err(OragError::SchemaTooNew {
            found: current,
            supported: target,
        });
    }
    if current == target {
        return Ok(current);
    }
    let foreign_keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = apply(conn, db_path, migrations, current > 0);
    let restored = conn.pragma_update(None, "foreign_keys", foreign_keys);
    let version = result?;
    restored?;
    Ok(version)
}

/// How long a migration waits for another process's migration to finish.
const MIGRATION_WAIT: Duration = Duration::from_secs(300);

/// Runs the steps. A backup from an attempt that then fails before any step
/// commits is removed again (the database is unchanged), so a migration that
/// fails on every start does not fill the disk with backups.
fn apply(
    conn: &Connection,
    db_path: &Path,
    migrations: &[&str],
    back_up_first: bool,
) -> Result<u32> {
    let mut progress = Progress {
        backup: None,
        needs_backup: back_up_first,
        committed: false,
    };
    let result = run_steps(conn, db_path, migrations, &mut progress);
    if let (Err(_), Some(backup), false) = (&result, &progress.backup, progress.committed) {
        let _ = std::fs::remove_file(backup);
    }
    result
}

struct Progress {
    backup: Option<PathBuf>,
    needs_backup: bool,
    committed: bool,
}

fn run_steps(
    conn: &Connection,
    db_path: &Path,
    migrations: &[&str],
    progress: &mut Progress,
) -> Result<u32> {
    let target = migrations.len() as u32;
    loop {
        let tx = begin_immediate(conn)?;
        let version = schema_version(&tx)?;
        if version > target {
            return Err(OragError::SchemaTooNew {
                found: version,
                supported: target,
            });
        }
        if version == target {
            return Ok(version);
        }
        if progress.needs_backup {
            drop(tx); // VACUUM INTO cannot run inside a transaction
            let backup = backup_path(db_path, target)?;
            progress.backup = back_up(conn, &backup, target)?.then_some(backup);
            progress.needs_backup = false;
            continue; // re-read the version under the lock
        }
        tx.execute_batch(migrations[version as usize])?;
        if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(OragError::Internal(format!(
                "migration {} leaves rows that break foreign keys",
                version + 1
            )));
        }
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
        progress.committed = true;
    }
}

/// `BEGIN IMMEDIATE`, retried while another process holds the write lock
/// (a long migration or backup) for up to `MIGRATION_WAIT`.
pub(crate) fn begin_immediate(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(super::retry_busy(MIGRATION_WAIT, || {
        Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
    })?)
}

/// `PRAGMA user_version`, which SQLite stores signed; a negative value was
/// not written by orag.
pub(crate) fn schema_version(conn: &Connection) -> Result<u32> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    u32::try_from(version).map_err(|_| not_orag_version(version))
}

pub(crate) fn not_orag_version(version: i64) -> OragError {
    OragError::InvalidInput(format!("not an orag database (schema version {version})"))
}

/// `<unix seconds>-<pid>-<n>`, unique per call within and across processes.
fn unique_suffix() -> String {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{seconds}-{}-{call}", std::process::id())
}

/// `<db>.pre-schema-<target>.<unix seconds>-<pid>-<n>.bak`: every upgrade
/// attempt gets its own file, so a backup left by an earlier attempt (or an
/// earlier database at the same path) never stands in for this one.
fn backup_path(db_path: &Path, target: u32) -> Result<PathBuf> {
    let file_name = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            OragError::InvalidInput(format!("{} has no UTF-8 file name", db_path.display()))
        })?;
    let unique = unique_suffix();
    Ok(db_path.with_file_name(format!("{file_name}.pre-schema-{target}.{unique}.bak")))
}

/// Writes the pre-upgrade backup and says whether it was kept. A copy that
/// already holds the target schema (another process upgraded first) is not.
/// The copy is taken without the write lock (SQLite cannot `VACUUM INTO`
/// inside a transaction), so a write another process commits between the
/// copy and the first step is not in it; v0.1 runs one orag per database.
fn back_up(conn: &Connection, backup: &Path, target: u32) -> Result<bool> {
    let temp = TempCopy::of(conn, backup)?;
    let reader = Connection::open_with_flags(&temp.0, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let copied = schema_version(&reader)?;
    drop(reader);
    if copied >= target {
        return Ok(false);
    }
    publish(&temp.0, backup)?;
    Ok(true)
}

/// Writes a complete copy of the database behind `conn` to `dest`: the copy
/// goes to a temporary file first and only a finished copy appears under
/// `dest`, so a full disk or a crash never leaves a partial file there.
/// Fails with `AlreadyExists` instead of overwriting `dest`.
pub(crate) fn copy_database(conn: &Connection, dest: &Path) -> Result<()> {
    super::sqlite_path(dest)?; // a name SQLite and the caller can both use
    let temp = TempCopy::of(conn, dest)?;
    Ok(publish(&temp.0, dest)?)
}

/// A `VACUUM INTO` copy next to `near`, removed on drop.
struct TempCopy(PathBuf);

impl TempCopy {
    fn of(conn: &Connection, near: &Path) -> Result<TempCopy> {
        let name = near
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let path = near.with_file_name(format!(".{name}.tmp-{}", unique_suffix()));
        let temp = TempCopy(path);
        conn.execute("VACUUM INTO ?1", [super::sqlite_path(&temp.0)?])?;
        Ok(temp)
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Puts the finished file `from` at `to` without overwriting: a hard link
/// where the filesystem has them, otherwise a copy into a new file. The data
/// is synced before it gets the final name and the directory entry after,
/// so a power loss never leaves a short file under `to`.
fn publish(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::File::open(from)?.sync_all()?;
    match std::fs::hard_link(from, to) {
        Err(err) if err.kind() != io::ErrorKind::AlreadyExists => copy_no_clobber(from, to)?,
        other => other?,
    }
    sync_parent(to)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => std::fs::File::open(dir)?.sync_all(),
        _ => Ok(()),
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(()) // Windows has no directory fsync; NTFS journals the entry
}

/// Copies `from` into a new file `to` (`AlreadyExists` if it is there) and
/// syncs it; a failed copy removes what it wrote.
pub(crate) fn copy_no_clobber(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = std::fs::File::open(from)?; // before `to` exists
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let written = io::copy(&mut source, &mut out).and_then(|_| out.sync_all());
    if written.is_err() {
        let _ = std::fs::remove_file(to);
    }
    written
}
