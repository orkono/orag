//! SQLite storage (D-003). One writer connection behind a mutex; readers open
//! short-lived read-only connections (WAL allows concurrent readers).
//! All methods block: async callers must use `tokio::task::spawn_blocking`.

pub mod migrations;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{OragError, Result};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Store {
    path: PathBuf,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first caller arrives in Task 8")
    )]
    writer: Mutex<Connection>,
    schema_version: u32,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        register_sqlite_vec()?;
        let path = absolute(path)?;
        let in_database = |err: OragError| match err {
            OragError::Storage(cause) => OragError::Database {
                path: path.display().to_string(),
                cause,
            },
            OragError::Io(err) => OragError::Io(std::io::Error::new(
                err.kind(),
                format!("database {}: {err}", path.display()),
            )),
            OragError::InvalidInput(message) => {
                OragError::InvalidInput(format!("database {}: {message}", path.display()))
            }
            other => other,
        };
        sqlite_path(&path)?; // a name SQLite can back up and upgrade later
        let mut writer = open_connection(&path, false).map_err(in_database)?;
        check_ownership(&writer).map_err(in_database)?;
        let schema_version = migrations::migrate(&mut writer, &path).map_err(in_database)?;
        Ok(Store {
            path,
            writer: Mutex::new(writer),
            schema_version,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Runs `f` on the writer connection. A transaction `f` leaves open is
    /// rolled back right away, so the WAL write lock is never held between
    /// calls; if `f` returned success anyway, that is an error, since its
    /// writes were not saved. If the rollback itself fails, `f`'s own error
    /// is kept. A panic in an earlier `f`
    /// leaves the mutex poisoned and possibly a transaction open; the lock is
    /// recovered and that transaction rolled back, so one bad write cannot
    /// disable the store for the rest of the process.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first caller arrives in Task 8")
    )]
    pub(crate) fn write<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = RollbackOnUnwind(self.writer.lock().unwrap_or_else(|poisoned| {
            self.writer.clear_poison();
            poisoned.into_inner()
        }));
        if !conn.0.is_autocommit() {
            conn.0.execute_batch("ROLLBACK")?;
        }
        let result = f(&mut conn.0);
        if conn.0.is_autocommit() {
            return result;
        }
        let rollback = conn.0.execute_batch("ROLLBACK");
        match (result, rollback) {
            (Err(err), _) => Err(err),
            (Ok(_), Err(err)) => Err(err.into()),
            (Ok(_), Ok(())) => Err(OragError::Internal(
                "a write returned success with its transaction still open; it was rolled back"
                    .into(),
            )),
        }
    }

    /// Consistent online backup via `VACUUM INTO` (D-004) on its own read-only
    /// connection, so writers are not blocked. Only a complete copy ever
    /// appears at `dest`, and an existing `dest` is never overwritten.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let exists = || OragError::InvalidInput(format!("{} already exists", dest.display()));
        if dest.exists() {
            return Err(exists());
        }
        let reader = open_connection(&self.path, true)?;
        migrations::copy_database(&reader, dest).map_err(|err| match err {
            OragError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => exists(),
            other => other,
        })
    }
}

/// The writer lock; if a write panics, its open transaction is rolled back
/// while unwinding, so the WAL write lock is released at once.
struct RollbackOnUnwind<'a>(std::sync::MutexGuard<'a, Connection>);

impl Drop for RollbackOnUnwind<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
    }
}

/// orag's `PRAGMA application_id` ('ORAG'), set by the first migration.
const APPLICATION_ID: i64 = 0x4F52_4147;

/// Refuses a file another program owns: an unversioned file that already has
/// tables, or a versioned one without orag's application id.
fn check_ownership(conn: &Connection) -> Result<()> {
    // One statement reads all three from one snapshot, so a migration that
    // another process commits meanwhile is seen whole or not at all.
    let (version, id, tables): (i64, i64, i64) = conn.query_row(
        "SELECT (SELECT user_version FROM pragma_user_version), \
                (SELECT application_id FROM pragma_application_id), \
                (SELECT COUNT(*) FROM sqlite_schema WHERE substr(name, 1, 7) <> 'sqlite_')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if version < 0 {
        return Err(migrations::not_orag_version(version));
    }
    let foreign = if version == 0 {
        tables > 0
    } else {
        id != APPLICATION_ID
    };
    if foreign {
        return Err(OragError::InvalidInput(
            "not an orag database; choose another file".into(),
        ));
    }
    Ok(())
}

/// `path` made absolute. The bundled SQLite is built with `SQLITE_USE_URI`,
/// so a relative name starting with `file:` would be read as a URI; an
/// absolute path starts with `/` (or a drive) and is always a plain file.
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(std::path::absolute(path)?)
}

/// `path` as the absolute UTF-8 text SQLite takes for `VACUUM INTO`. A path
/// that is not UTF-8 cannot be named exactly, so it is rejected instead of
/// rewritten.
fn sqlite_path(path: &Path) -> Result<String> {
    let path = absolute(path)?;
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| OragError::InvalidInput(format!("{} is not a UTF-8 path", path.display())))
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection> {
    // NO_MUTEX is rusqlite's own default (rusqlite 0.40 lib.rs): `Connection` is
    // `Send` but not `Sync`, the writer sits behind a `Mutex`, and every read
    // opens its own connection, so no connection is shared between threads.
    // URI parsing is compiled in (SQLITE_USE_URI); callers pass absolute paths,
    // which are never read as URIs.
    let base = OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let flags = if read_only {
        base | OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        base | OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    if !read_only {
        let mode = enable_wal(&conn)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(OragError::Internal(format!(
                "could not enable WAL (got {mode})"
            )));
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    }
    Ok(conn)
}

/// Switching to WAL can return SQLITE_BUSY without calling the busy handler
/// when another process is opening the same new database, so it is retried
/// for as long as the busy timeout.
fn enable_wal(conn: &Connection) -> Result<String> {
    Ok(retry_busy(BUSY_TIMEOUT, || {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
    })?)
}

/// Runs `f` again every 10 ms while it fails with SQLITE_BUSY, until `limit`
/// has passed; then its last result is returned.
pub(crate) fn retry_busy<T>(
    limit: Duration,
    mut f: impl FnMut() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let started = std::time::Instant::now();
    loop {
        match f() {
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::DatabaseBusy && started.elapsed() < limit =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

type ExtensionInit = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut std::os::raw::c_char,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

/// Registers sqlite-vec for every connection opened afterwards in this
/// process. The outcome is kept, so a failed registration fails every open
/// instead of surfacing later as "no such module: vec0".
fn register_sqlite_vec() -> Result<()> {
    static REGISTERED: OnceLock<std::os::raw::c_int> = OnceLock::new();
    let rc = *REGISTERED.get_or_init(|| {
        // SAFETY: `sqlite3_vec_init` is the extension entry point compiled into
        // this binary by the sqlite-vec crate; its signature matches `ExtensionInit`
        // (the bindgen type of `sqlite3_auto_extension`'s argument). This is the
        // registration used by sqlite-vec 0.1.9's own test (src/lib.rs), and it was
        // run during planning together with the partition-key query of Task 10.
        unsafe {
            let init = std::mem::transmute::<*const (), ExtensionInit>(
                sqlite_vec::sqlite3_vec_init as *const (),
            );
            rusqlite::ffi::sqlite3_auto_extension(Some(init))
        }
    });
    if rc == rusqlite::ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(OragError::Internal(format!(
            "registering sqlite-vec failed (SQLite code {rc})"
        )))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Store;

    pub(crate) fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("orag.db")).expect("open store");
        (dir, store)
    }

    /// Plain connection for assertions (sqlite-vec is auto-registered by `Store::open`).
    pub(crate) fn conn(store: &Store) -> rusqlite::Connection {
        rusqlite::Connection::open(store.path()).expect("open connection")
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{conn, temp_store};
    use super::*;

    #[test]
    fn fresh_database_has_schema_and_default_collection() {
        let (_dir, store) = temp_store();
        assert_eq!(store.schema_version(), migrations::SUPPORTED_SCHEMA_VERSION);
        let conn = conn(&store);
        let name: String = conn
            .query_row("SELECT name FROM collections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "default");
    }

    #[test]
    fn bundled_sqlite_has_wal_reset_fix() {
        let (_dir, store) = temp_store();
        let version: String = conn(&store)
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))
            .unwrap();
        let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
        assert!(parts >= vec![3, 51, 3], "bundled SQLite {version} < 3.51.3");
    }

    #[test]
    fn sqlite_vec_is_the_pinned_stable_release() {
        let (_dir, store) = temp_store();
        let version: String = conn(&store)
            .query_row("SELECT vec_version()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, "v0.1.9");
    }

    #[test]
    fn reopening_is_idempotent() {
        let (dir, store) = temp_store();
        drop(store);
        let again = Store::open(&dir.path().join("orag.db")).unwrap();
        let count: i64 = conn(&again)
            .query_row("SELECT COUNT(*) FROM collections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn newer_schema_is_refused() {
        let (dir, store) = temp_store();
        store
            .write(|conn| Ok(conn.pragma_update(None, "user_version", 99)?))
            .unwrap();
        drop(store);
        let err = Store::open(&dir.path().join("orag.db")).err().unwrap();
        assert!(
            matches!(err, OragError::SchemaTooNew { found: 99, .. }),
            "{err}"
        );
    }

    #[test]
    fn pending_migrations_on_existing_db_take_a_backup_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(&mut conn, &path, &["CREATE TABLE a (x);"]).unwrap();
        let version = migrations::migrate_with(
            &mut conn,
            &path,
            &["CREATE TABLE a (x);", "CREATE TABLE b (y);"],
        )
        .unwrap();
        assert_eq!(version, 2);
        assert_eq!(backups(dir.path()).len(), 1);
    }

    #[test]
    fn backup_produces_a_consistent_copy() {
        let (dir, store) = temp_store();
        let dest = dir.path().join("copy.db");
        store.backup_to(&dest).unwrap();
        let copy = rusqlite::Connection::open(&dest).unwrap();
        let name: String = copy
            .query_row("SELECT name FROM collections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "default");
        assert!(
            store.backup_to(&dest).is_err(),
            "must not overwrite an existing file"
        );
    }

    #[test]
    fn concurrent_first_opens_all_succeed() {
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("orag.db");
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || Store::open(&path).map(|s| s.schema_version()))
                })
                .collect();
            for handle in handles {
                let version = handle.join().unwrap().expect("concurrent open");
                assert_eq!(version, migrations::SUPPORTED_SCHEMA_VERSION);
            }
        }
    }

    #[test]
    fn a_failed_migration_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(&mut conn, &path, &["CREATE TABLE a (x);"]).unwrap();
        let broken = ["CREATE TABLE a (x);", "CREATE TABLE b (y);", "NOT SQL;"];
        assert!(migrations::migrate_with(&mut conn, &path, &broken).is_err());
        let fixed = [
            "CREATE TABLE a (x);",
            "CREATE TABLE b (y);",
            "CREATE TABLE c (z);",
        ];
        assert_eq!(
            migrations::migrate_with(&mut conn, &path, &fixed).unwrap(),
            3
        );
    }

    #[test]
    fn a_chunk_must_belong_to_its_documents_collection() {
        let (_dir, store) = temp_store();
        let result = store.write(|conn| {
            conn.execute_batch(
                "INSERT INTO collections (name) VALUES ('other');
                 INSERT INTO sources VALUES ('h', 1, x'00');
                 INSERT INTO documents (collection_id, source_sha256, format, status)
                     VALUES (1, 'h', 'plain_text', 'ready');",
            )?;
            conn.execute(
                "INSERT INTO chunks (document_id, collection_id, ordinal, heading_path, text, token_count)
                 VALUES (1, 2, 0, '[]', 'x', 1)",
                [],
            )?;
            Ok(())
        });
        assert!(
            result.is_err(),
            "a chunk filed under another collection must be rejected"
        );
    }

    #[test]
    fn storage_errors_print_their_cause_once() {
        let err = OragError::from(rusqlite::Error::InvalidQuery);
        let shown = format!("{:#}", anyhow::Error::from(err));
        assert_eq!(
            shown.matches("Query is not read-only").count(),
            1,
            "{shown}"
        );
    }

    #[test]
    fn open_errors_name_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("orag.db");
        let err = Store::open(&path).err().unwrap().to_string();
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    #[test]
    fn a_panic_in_a_writer_does_not_break_the_store() {
        let (_dir, store) = temp_store();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store
                .write(|conn| -> Result<()> {
                    conn.execute_batch("BEGIN; INSERT INTO collections (name) VALUES ('half');")?;
                    panic!("boom");
                })
                .ok()
        }));
        assert!(panicked.is_err());
        let names: i64 = store
            .write(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM collections", [], |r| r.get(0))?))
            .expect("store still usable");
        assert_eq!(names, 1, "the half-done transaction was rolled back");
    }

    #[cfg(unix)]
    #[test]
    fn backup_rejects_paths_sqlite_cannot_name() {
        use std::os::unix::ffi::OsStrExt;
        let (dir, store) = temp_store();
        let odd = dir.path().join(std::ffi::OsStr::from_bytes(b"b\xff.db"));
        assert!(matches!(
            store.backup_to(&odd),
            Err(OragError::InvalidInput(_))
        ));
        let uri = dir.path().join("file:x.db?mode=memory");
        store.backup_to(&uri).unwrap();
        assert!(
            uri.exists(),
            "a name that looks like a URI is still a plain file"
        );
    }

    #[test]
    fn a_schema_raised_during_migration_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut newer = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(
            &mut newer,
            &path,
            &["CREATE TABLE a (x);", "CREATE TABLE b (y);"],
        )
        .unwrap();
        let mut older = rusqlite::Connection::open(&path).unwrap();
        let err =
            migrations::migrate_with(&mut older, &path, &["CREATE TABLE a (x);"]).unwrap_err();
        assert!(
            matches!(
                err,
                OragError::SchemaTooNew {
                    found: 2,
                    supported: 1
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_failed_writer_does_not_hold_the_write_lock() {
        let (_dir, store) = temp_store();
        let failed = store.write(|conn| -> Result<()> {
            conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO collections (name) VALUES ('x');")?;
            Err(OragError::Internal("stop".into()))
        });
        assert!(failed.is_err());
        let other = conn(&store);
        other.busy_timeout(std::time::Duration::ZERO).unwrap();
        other
            .execute("INSERT INTO collections (name) VALUES ('y')", [])
            .expect("the failed writer released its transaction");
    }

    #[test]
    fn uri_like_relative_paths_are_plain_files() {
        let name = sqlite_path(Path::new("file:x.db?mode=memory")).unwrap();
        assert!(Path::new(&name).is_absolute(), "{name}");
        assert!(name.ends_with("file:x.db?mode=memory"));
    }

    #[test]
    fn open_errors_keep_the_storage_cause() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("orag.db");
        let err = Store::open(&path).err().unwrap();
        assert!(matches!(err, OragError::Database { .. }), "{err:?}");
    }

    #[test]
    fn the_backup_is_the_complete_pre_upgrade_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(&mut conn, &path, &["CREATE TABLE a (x);"]).unwrap();
        let two = ["CREATE TABLE a (x);", "CREATE TABLE b (y);"];
        assert_eq!(migrations::migrate_with(&mut conn, &path, &two).unwrap(), 2);
        let copy = rusqlite::Connection::open(&backups(dir.path())[0]).unwrap();
        let version: u32 = copy
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(
            version, 1,
            "the kept backup is the complete pre-upgrade database"
        );
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0, "temporary backup files are removed");
    }

    #[test]
    fn ok_with_an_open_transaction_is_an_error() {
        let (_dir, store) = temp_store();
        let result = store.write(|conn| {
            conn.execute_batch("BEGIN; INSERT INTO collections (name) VALUES ('x');")?;
            Ok(())
        });
        assert!(matches!(result, Err(OragError::Internal(_))), "{result:?}");
    }

    #[test]
    fn migrations_run_without_foreign_key_actions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let v1 = "CREATE TABLE p (id INTEGER PRIMARY KEY);
                  CREATE TABLE c (pid INTEGER REFERENCES p(id) ON DELETE CASCADE);
                  INSERT INTO p VALUES (1); INSERT INTO c VALUES (1);";
        let rebuild = "CREATE TABLE p2 (id INTEGER PRIMARY KEY, note TEXT);
                       INSERT INTO p2 (id) SELECT id FROM p;
                       DROP TABLE p; ALTER TABLE p2 RENAME TO p;";
        migrations::migrate_with(&mut conn, &path, &[v1, rebuild]).unwrap();
        let children: i64 = conn
            .query_row("SELECT COUNT(*) FROM c", [], |r| r.get(0))
            .unwrap();
        assert_eq!(children, 1, "a table rebuild must not cascade");
        let on: bool = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(on, "foreign keys are restored after migrating");
    }

    #[test]
    fn a_migration_that_breaks_foreign_keys_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        let v1 = "CREATE TABLE p (id INTEGER PRIMARY KEY);
                  CREATE TABLE c (pid INTEGER REFERENCES p(id));
                  INSERT INTO p VALUES (1); INSERT INTO c VALUES (1);";
        let orphan = "DELETE FROM p;";
        assert!(migrations::migrate_with(&mut conn, &path, &[v1, orphan]).is_err());
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1, "the broken migration was rolled back");
    }

    #[test]
    fn opening_a_current_database_does_not_wait_for_writers() {
        let (dir, store) = temp_store();
        let holder = conn(&store);
        holder.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let started = std::time::Instant::now();
        Store::open(&dir.path().join("orag.db")).expect("open while another writer holds the lock");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn copying_without_hard_links_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&from, b"data").unwrap();
        migrations::copy_no_clobber(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"data");
        let again = migrations::copy_no_clobber(&from, &to).unwrap_err();
        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists);
    }

    // macOS (APFS) refuses non-UTF-8 file names itself; Linux allows them.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_database_path_is_refused_up_front() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9.db"));
        // It could never be backed up or upgraded (SQLite names files in UTF-8).
        assert!(matches!(
            Store::open(&path),
            Err(OragError::InvalidInput(_))
        ));
    }

    fn backups(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".bak"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_new_database_needing_several_migrations_gets_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        let three = [
            "CREATE TABLE a (x);",
            "CREATE TABLE b (y);",
            "CREATE TABLE c (z);",
        ];
        assert_eq!(
            migrations::migrate_with(&mut conn, &path, &three).unwrap(),
            3
        );
        assert!(backups(dir.path()).is_empty());
    }

    #[test]
    fn every_upgrade_takes_its_own_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(&mut conn, &path, &["CREATE TABLE a (x);"]).unwrap();
        let broken = ["CREATE TABLE a (x);", "NOT SQL;"];
        assert!(migrations::migrate_with(&mut conn, &path, &broken).is_err());
        conn.execute_batch("INSERT INTO a VALUES ('newer data')")
            .unwrap();
        let fixed = ["CREATE TABLE a (x);", "CREATE TABLE b (y);"];
        migrations::migrate_with(&mut conn, &path, &fixed).unwrap();
        let latest = rusqlite::Connection::open(backups(dir.path()).last().unwrap()).unwrap();
        let rows: i64 = latest
            .query_row("SELECT COUNT(*) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rows, 1,
            "the backup holds the data as it was right before this upgrade"
        );
    }

    #[test]
    fn a_panic_releases_the_write_lock_at_once() {
        let (_dir, store) = temp_store();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.write(|conn| -> Result<()> {
                conn.execute_batch(
                    "BEGIN IMMEDIATE; INSERT INTO collections (name) VALUES ('x');",
                )?;
                panic!("boom");
            })
        }));
        let other = conn(&store);
        other.busy_timeout(std::time::Duration::ZERO).unwrap();
        other
            .execute("INSERT INTO collections (name) VALUES ('y')", [])
            .expect("lock released");
    }

    #[test]
    fn another_apps_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE notes (t);")
            .unwrap();
        let err = Store::open(&path).err().unwrap();
        assert!(matches!(err, OragError::InvalidInput(_)), "{err}");
        let tagged = dir.path().join("tagged.db");
        let other = rusqlite::Connection::open(&tagged).unwrap();
        other
            .execute_batch("PRAGMA application_id = 7; PRAGMA user_version = 1;")
            .unwrap();
        drop(other);
        assert!(matches!(
            Store::open(&tagged),
            Err(OragError::InvalidInput(_))
        ));
    }

    #[test]
    fn a_negative_schema_version_is_refused_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", -1).unwrap();
        let err = migrations::migrate_with(&mut conn, &path, &["SELECT 1;"]).unwrap_err();
        assert!(matches!(err, OragError::InvalidInput(_)), "{err}");
    }

    #[test]
    fn a_failed_copy_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("b");
        assert!(migrations::copy_no_clobber(&dir.path().join("missing"), &to).is_err());
        assert!(!to.exists());
    }

    #[test]
    fn repeated_failed_upgrades_do_not_pile_up_backups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::migrate_with(&mut conn, &path, &["CREATE TABLE a (x);"]).unwrap();
        let broken = ["CREATE TABLE a (x);", "NOT SQL;"];
        for _ in 0..3 {
            assert!(migrations::migrate_with(&mut conn, &path, &broken).is_err());
        }
        assert!(
            backups(dir.path()).is_empty(),
            "nothing changed, so no backup is kept"
        );
    }

    #[test]
    fn tables_named_like_sqlite_still_count_as_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE SQLiteCache (t);")
            .unwrap();
        assert!(matches!(
            Store::open(&path),
            Err(OragError::InvalidInput(_))
        ));
    }

    #[test]
    fn refusals_name_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", -1)
            .unwrap();
        let err = Store::open(&path).err().unwrap().to_string();
        assert!(err.contains("t.db"), "{err}");
    }
}
