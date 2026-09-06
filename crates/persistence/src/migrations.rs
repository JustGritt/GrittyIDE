//! Forward-only, numbered migrations (ADR-004, ADR-014).
//!
//! Each migration is one SQL file applied inside one transaction together with
//! its `schema_version` row, so a failure leaves the database exactly as it
//! was (SQLite DDL is transactional). The runner owns the `schema_version`
//! table; migrations own the domain tables. A database whose version is above
//! what this build knows is refused, never downgraded.

use rusqlite::{Connection, params};

use crate::error::PersistenceError;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every migration, in order. Append only; never edit an applied one.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "workstreams",
        sql: include_str!("migrations/0002_workstreams.sql"),
    },
    Migration {
        version: 3,
        name: "ownership",
        sql: include_str!("migrations/0003_ownership.sql"),
    },
    Migration {
        version: 4,
        name: "agent_links",
        sql: include_str!("migrations/0004_agent_links.sql"),
    },
];

/// The schema version this build expects after migrating.
pub const SCHEMA_VERSION: i64 = 4;
const _: () = assert!(MIGRATIONS[MIGRATIONS.len() - 1].version == SCHEMA_VERSION);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub from: i64,
    pub to: i64,
}

impl MigrationReport {
    pub fn applied(&self) -> i64 {
        self.to - self.from
    }
}

/// The highest applied version, `0` for a database without one.
pub fn current_version(conn: &Connection) -> Result<i64, PersistenceError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )?;
    let version = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(version)
}

/// Apply every migration in `migrations` above the current version, one
/// transaction each. `now_ms` is recorded as `applied_at`.
pub fn migrate(
    conn: &mut Connection,
    migrations: &[Migration],
    now_ms: i64,
) -> Result<MigrationReport, PersistenceError> {
    debug_assert!(
        migrations
            .iter()
            .enumerate()
            .all(|(index, migration)| migration.version == index as i64 + 1),
        "migrations must be numbered 1..=n without gaps"
    );
    let from = current_version(conn)?;
    let supported = migrations.last().map_or(0, |migration| migration.version);
    if from > supported {
        return Err(PersistenceError::NewerSchema {
            found: from,
            supported,
        });
    }
    for migration in migrations.iter().filter(|m| m.version > from) {
        let tx = conn.transaction()?;
        let applied = tx.execute_batch(migration.sql).and_then(|()| {
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                params![migration.version, now_ms],
            )
            .map(|_| ())
        });
        if let Err(source) = applied {
            // Dropping `tx` rolls the whole migration back.
            return Err(PersistenceError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            });
        }
        tx.commit()?;
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applied migration"
        );
    }
    Ok(MigrationReport {
        from,
        to: supported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    #[test]
    fn migrations_are_numbered_contiguously_from_one() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(migration.version, index as i64 + 1);
            let opens_transaction = migration
                .sql
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("--"))
                .any(|line| line.to_ascii_uppercase().starts_with("BEGIN"));
            assert!(
                !opens_transaction,
                "{}: the runner owns the transaction",
                migration.name
            );
        }
    }

    #[test]
    fn migrates_an_empty_database_to_the_latest_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
        let report = migrate(&mut conn, MIGRATIONS, 1).unwrap();
        assert_eq!(
            report,
            MigrationReport {
                from: 0,
                to: SCHEMA_VERSION
            }
        );
        assert_eq!(current_version(&conn).unwrap(), SCHEMA_VERSION);
        for table in [
            "workspace",
            "session",
            "window_state",
            "workspace_tab",
            "buffer_snapshot",
            "operation_record",
            "workstream",
            "decision",
            "ownership_lease",
            "ownership_conflict",
            "workstream_agent_link",
        ] {
            assert!(table_exists(&conn, table), "{table}");
        }
        let again = migrate(&mut conn, MIGRATIONS, 2).unwrap();
        assert_eq!(again.applied(), 0);
    }

    #[test]
    fn migrates_from_each_prior_version() {
        for applied in 0..MIGRATIONS.len() {
            let mut conn = Connection::open_in_memory().unwrap();
            migrate(&mut conn, &MIGRATIONS[..applied], 1).unwrap();
            assert_eq!(current_version(&conn).unwrap(), applied as i64);
            let report = migrate(&mut conn, MIGRATIONS, 2).unwrap();
            assert_eq!(report.from, applied as i64);
            assert_eq!(report.to, SCHEMA_VERSION);
        }
    }

    #[test]
    fn a_failing_migration_is_rolled_back_completely() {
        let mut conn = Connection::open_in_memory().unwrap();
        let mut migrations = MIGRATIONS.to_vec();
        migrations.push(Migration {
            version: SCHEMA_VERSION + 1,
            name: "broken",
            sql: "CREATE TABLE half_done (x INTEGER); THIS IS NOT SQL;",
        });
        let error = migrate(&mut conn, &migrations, 1).unwrap_err();
        assert!(
            matches!(error, PersistenceError::Migration { version, name: "broken", .. } if version == SCHEMA_VERSION + 1),
            "{error}"
        );
        assert!(!error.is_corrupt());
        assert_eq!(
            current_version(&conn).unwrap(),
            SCHEMA_VERSION,
            "good migrations stay applied"
        );
        assert!(
            !table_exists(&conn, "half_done"),
            "partial DDL was rolled back"
        );
        assert!(table_exists(&conn, "session"));
    }

    #[test]
    fn a_newer_schema_is_refused_without_changes() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, MIGRATIONS, 1).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, 1)",
            [SCHEMA_VERSION + 5],
        )
        .unwrap();
        let error = migrate(&mut conn, MIGRATIONS, 2).unwrap_err();
        assert!(
            matches!(error, PersistenceError::NewerSchema { found, supported } if found == SCHEMA_VERSION + 5 && supported == SCHEMA_VERSION),
            "{error}"
        );
    }
}
