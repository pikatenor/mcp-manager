use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use super::store::StoreError;

/// A one-time startup job. `run` must be idempotent: a failure leaves the
/// ledger untouched, so the next launch runs the job again.
pub struct Migration<'a> {
    pub name: &'static str,
    pub run: Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>,
}

/// What `run_pending` did with each registered job, in registration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    Applied(&'static str),
    Skipped(&'static str),
    Failed { name: &'static str, error: String },
}

/// Append-only record of applied migration names, so update-time jobs run
/// exactly once per data directory.
pub struct MigrationLog {
    conn: Mutex<Connection>,
}

impl MigrationLog {
    /// Open (or create) the ledger beside the other store files.
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Database(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS migrations (
                name TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn is_applied(&self, name: &str) -> Result<bool, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?;
        let found = conn
            .query_row("SELECT 1 FROM migrations WHERE name = ?1", [name], |_| Ok(()))
            .optional()
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(found.is_some())
    }

    pub fn mark_applied(&self, name: &str) -> Result<(), StoreError> {
        let applied_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO migrations (name, applied_at) VALUES (?1, ?2)",
            params![name, applied_at],
        )
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }
}

/// Runs unapplied jobs in registration order and marks them only after
/// success, so a failed job retries on the next launch. Jobs keep running
/// after an earlier failure; ledger errors surface as failed outcomes.
pub async fn run_pending(
    log: &MigrationLog,
    migrations: Vec<Migration<'_>>,
) -> Vec<MigrationOutcome> {
    let mut outcomes = Vec::new();
    for Migration { name, run } in migrations {
        let applied = match log.is_applied(name) {
            Ok(applied) => applied,
            Err(error) => {
                outcomes.push(MigrationOutcome::Failed {
                    name,
                    error: error.to_string(),
                });
                continue;
            }
        };
        if applied {
            outcomes.push(MigrationOutcome::Skipped(name));
            continue;
        }
        match run.await {
            Ok(()) => match log.mark_applied(name) {
                Ok(()) => outcomes.push(MigrationOutcome::Applied(name)),
                Err(error) => outcomes.push(MigrationOutcome::Failed {
                    name,
                    error: error.to_string(),
                }),
            },
            Err(error) => outcomes.push(MigrationOutcome::Failed { name, error }),
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn temp_log() -> (MigrationLog, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let log = MigrationLog::open_sqlite(&dir.path().join("migrations.db")).unwrap();
        (log, dir)
    }

    fn job(
        name: &'static str,
        ran: Arc<Mutex<Vec<&'static str>>>,
        ok: bool,
    ) -> Migration<'static> {
        Migration {
            name,
            run: Box::pin(async move {
                ran.lock().unwrap().push(name);
                if ok {
                    Ok(())
                } else {
                    Err(format!("{name} exploded"))
                }
            }),
        }
    }

    #[tokio::test]
    async fn pending_jobs_run_in_order_and_are_marked() {
        let (log, _dir) = temp_log();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let outcomes = run_pending(
            &log,
            vec![job("0001", ran.clone(), true), job("0002", ran.clone(), true)],
        )
        .await;
        assert_eq!(
            outcomes,
            vec![MigrationOutcome::Applied("0001"), MigrationOutcome::Applied("0002")]
        );
        assert_eq!(*ran.lock().unwrap(), vec!["0001", "0002"]);
        assert!(log.is_applied("0001").unwrap());
        assert!(log.is_applied("0002").unwrap());
    }

    #[tokio::test]
    async fn applied_jobs_are_skipped() {
        let (log, _dir) = temp_log();
        log.mark_applied("0001").unwrap();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let outcomes = run_pending(&log, vec![job("0001", ran.clone(), true)]).await;
        assert_eq!(outcomes, vec![MigrationOutcome::Skipped("0001")]);
        assert!(ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_job_is_not_marked_and_retries_next_run() {
        let (log, _dir) = temp_log();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let outcomes = run_pending(&log, vec![job("0001", ran.clone(), false)]).await;
        assert_eq!(
            outcomes,
            vec![MigrationOutcome::Failed {
                name: "0001",
                error: "0001 exploded".to_string()
            }]
        );
        assert!(!log.is_applied("0001").unwrap());

        let outcomes = run_pending(&log, vec![job("0001", ran.clone(), true)]).await;
        assert_eq!(outcomes, vec![MigrationOutcome::Applied("0001")]);
        assert_eq!(*ran.lock().unwrap(), vec!["0001", "0001"]);
    }

    #[tokio::test]
    async fn later_jobs_run_after_a_failure() {
        let (log, _dir) = temp_log();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let outcomes = run_pending(
            &log,
            vec![
                job("0001", ran.clone(), false),
                job("0002", ran.clone(), true),
            ],
        )
        .await;
        assert_eq!(*ran.lock().unwrap(), vec!["0001", "0002"]);
        assert!(matches!(
            &outcomes[0],
            MigrationOutcome::Failed { name: "0001", .. }
        ));
        assert_eq!(outcomes[1], MigrationOutcome::Applied("0002"));
    }

    #[test]
    fn ledger_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrations.db");
        {
            let log = MigrationLog::open_sqlite(&path).unwrap();
            log.mark_applied("0001").unwrap();
        }
        let log = MigrationLog::open_sqlite(&path).unwrap();
        assert!(log.is_applied("0001").unwrap());
        assert!(!log.is_applied("0002").unwrap());
    }
}
