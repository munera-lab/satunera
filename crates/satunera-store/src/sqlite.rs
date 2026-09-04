//! The SQLite implementation of [`Store`].
//!
//! WAL mode, `busy_timeout`, foreign keys on (ADR 0002). Every statement is a
//! `query!` macro, so the SQL is checked against the real schema at compile
//! time; `.sqlx/` carries the offline metadata so no teammate needs a live
//! database to build.

use std::str::FromStr;

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use satunera_contracts::{Language, ProblemId, Verdict};

use crate::error::StoreError;
use crate::id::{RunId, SubmissionId, UserId};
use crate::model::{
    NewRun, NewSubmission, NewUser, ProblemRow, Role, Run, Submission, SubmissionStatus, User,
    UserProgress,
};
use crate::store::Store;
use crate::MIGRATOR;

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Open (creating if missing) the database at `path`, apply migrations,
    /// and return a ready store.
    pub async fn open(path: &str) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str(path)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    /// A fresh in-memory database, for tests.
    pub async fn in_memory() -> Result<Self, StoreError> {
        // A pool of one: each in-memory connection is its own database.
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// A write transaction that takes the write lock up front.
    ///
    /// The default deferred `BEGIN` acquires the lock at the first write, so
    /// two connections doing check-then-write on the same row both read, and
    /// the loser dies with `SQLITE_BUSY_SNAPSHOT` — which `busy_timeout` does
    /// not retry. `BEGIN IMMEDIATE` serializes the whole transaction against
    /// other writers instead.
    async fn begin_immediate(&self) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>, StoreError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("UTC now always formats")
}

fn parse_ts(entity: &'static str, value: &str) -> Result<OffsetDateTime, StoreError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|e| StoreError::CorruptRow {
        entity,
        detail: format!("bad timestamp {value:?}: {e}"),
    })
}

fn parse_opt_ts(
    entity: &'static str,
    value: Option<&str>,
) -> Result<Option<OffsetDateTime>, StoreError> {
    value.map(|v| parse_ts(entity, v)).transpose()
}

fn corrupt(entity: &'static str, detail: impl Into<String>) -> StoreError {
    StoreError::CorruptRow {
        entity,
        detail: detail.into(),
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(db) if db.is_unique_violation()
    )
}

struct SubmissionRecord {
    id: String,
    user_id: String,
    problem_id: String,
    language: String,
    source_hash: String,
    source_uri: String,
    status: String,
    verdict: Option<String>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl SubmissionRecord {
    fn into_submission(self) -> Result<Submission, StoreError> {
        const E: &str = "submissions";
        Ok(Submission {
            id: self
                .id
                .parse()
                .map_err(|_| corrupt(E, format!("bad id {:?}", self.id)))?,
            user_id: self
                .user_id
                .parse()
                .map_err(|_| corrupt(E, format!("bad user_id {:?}", self.user_id)))?,
            problem_id: ProblemId::new(self.problem_id),
            language: Language::parse(&self.language)
                .ok_or_else(|| corrupt(E, format!("bad language {:?}", self.language)))?,
            source_hash: self.source_hash,
            source_uri: self.source_uri,
            status: SubmissionStatus::parse(&self.status)
                .ok_or_else(|| corrupt(E, format!("bad status {:?}", self.status)))?,
            verdict: self
                .verdict
                .as_deref()
                .map(|v| Verdict::parse(v).ok_or_else(|| corrupt(E, format!("bad verdict {v:?}"))))
                .transpose()?,
            created_at: parse_ts(E, &self.created_at)?,
            started_at: parse_opt_ts(E, self.started_at.as_deref())?,
            finished_at: parse_opt_ts(E, self.finished_at.as_deref())?,
        })
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn create_user(&self, user: NewUser) -> Result<User, StoreError> {
        let id = UserId::generate();
        let id_text = id.to_string();
        let role = user.role.as_str();
        let created_at = now();

        let result = sqlx::query!(
            "INSERT INTO users (id, handle, email, password_hash, role, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            id_text,
            user.handle,
            user.email,
            user.password_hash,
            role,
            created_at,
        )
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(User {
                id,
                handle: user.handle,
                email: user.email,
                password_hash: user.password_hash,
                role: user.role,
                created_at: parse_ts("users", &created_at)?,
            }),
            Err(e) if is_unique_violation(&e) => Err(StoreError::Conflict {
                entity: "users",
                detail: format!(
                    "handle {:?} or email {:?} already taken",
                    user.handle, user.email
                ),
            }),
            Err(e) => Err(e.into()),
        }
    }

    async fn user(&self, id: UserId) -> Result<User, StoreError> {
        let id_text = id.to_string();
        let row = sqlx::query!(
            "SELECT id, handle, email, password_hash, role, created_at
             FROM users WHERE id = ?",
            id_text,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "users",
            id: id_text,
        })?;

        Ok(User {
            id,
            handle: row.handle,
            email: row.email,
            password_hash: row.password_hash,
            role: Role::parse(&row.role)
                .ok_or_else(|| corrupt("users", format!("bad role {:?}", row.role)))?,
            created_at: parse_ts("users", &row.created_at)?,
        })
    }

    async fn user_by_handle(&self, handle: &str) -> Result<User, StoreError> {
        let row = sqlx::query!(
            "SELECT id, handle, email, password_hash, role, created_at
             FROM users WHERE handle = ?",
            handle,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "users",
            id: handle.to_string(),
        })?;

        Ok(User {
            id: row
                .id
                .parse()
                .map_err(|_| corrupt("users", format!("bad id {:?}", row.id)))?,
            handle: row.handle,
            email: row.email,
            password_hash: row.password_hash,
            role: Role::parse(&row.role)
                .ok_or_else(|| corrupt("users", format!("bad role {:?}", row.role)))?,
            created_at: parse_ts("users", &row.created_at)?,
        })
    }

    async fn replace_problems(&self, problems: Vec<ProblemRow>) -> Result<(), StoreError> {
        let mut tx = self.begin_immediate().await?;

        // Upsert rather than delete-and-reinsert: submissions foreign-key
        // into this table, and immediate FKs are checked per statement, so a
        // bulk DELETE would fail the moment any submission exists.
        for problem in &problems {
            let id = problem.id.as_str();
            let updated_at = problem
                .updated_at
                .format(&Rfc3339)
                .map_err(|e| corrupt("problems", format!("unformattable timestamp: {e}")))?;
            sqlx::query!(
                "INSERT INTO problems
                     (id, commit_sha, title, category, difficulty, manifest, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (id) DO UPDATE SET
                     commit_sha = excluded.commit_sha,
                     title = excluded.title,
                     category = excluded.category,
                     difficulty = excluded.difficulty,
                     manifest = excluded.manifest,
                     updated_at = excluded.updated_at",
                id,
                problem.commit_sha,
                problem.title,
                problem.category,
                problem.difficulty,
                problem.manifest,
                updated_at,
            )
            .execute(&mut *tx)
            .await?;
        }

        // Targeted delete of ids no longer in the content repo. Removing a
        // problem that already has submissions violates the foreign key and
        // fails the boot loudly, which beats orphaning the learner's history.
        let keep: std::collections::HashSet<&str> =
            problems.iter().map(|p| p.id.as_str()).collect();
        let existing = sqlx::query!("SELECT id FROM problems")
            .fetch_all(&mut *tx)
            .await?;
        for row in existing {
            if !keep.contains(row.id.as_str()) {
                sqlx::query!("DELETE FROM problems WHERE id = ?", row.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    async fn problem(&self, id: &ProblemId) -> Result<ProblemRow, StoreError> {
        let id_text = id.as_str();
        let row = sqlx::query!(
            "SELECT id, commit_sha, title, category, difficulty, manifest, updated_at
             FROM problems WHERE id = ?",
            id_text,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "problems",
            id: id_text.to_string(),
        })?;

        Ok(ProblemRow {
            id: ProblemId::new(row.id),
            commit_sha: row.commit_sha,
            title: row.title,
            category: row.category,
            difficulty: row.difficulty,
            manifest: row.manifest,
            updated_at: parse_ts("problems", &row.updated_at)?,
        })
    }

    async fn problems(&self) -> Result<Vec<ProblemRow>, StoreError> {
        let rows = sqlx::query!(
            "SELECT id, commit_sha, title, category, difficulty, manifest, updated_at
             FROM problems ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(ProblemRow {
                    id: ProblemId::new(row.id),
                    commit_sha: row.commit_sha,
                    title: row.title,
                    category: row.category,
                    difficulty: row.difficulty,
                    manifest: row.manifest,
                    updated_at: parse_ts("problems", &row.updated_at)?,
                })
            })
            .collect()
    }

    async fn create_submission(&self, submission: NewSubmission) -> Result<Submission, StoreError> {
        let id = SubmissionId::generate();
        let id_text = id.to_string();
        let user_id = submission.user_id.to_string();
        let problem_id = submission.problem_id.as_str().to_string();
        let language = submission.language.as_str();
        let created_at = now();

        sqlx::query!(
            "INSERT INTO submissions
                 (id, user_id, problem_id, language, source_hash, source_uri,
                  status, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'QUEUED', ?)",
            id_text,
            user_id,
            problem_id,
            language,
            submission.source_hash,
            submission.source_uri,
            created_at,
        )
        .execute(&self.pool)
        .await?;

        Ok(Submission {
            id,
            user_id: submission.user_id,
            problem_id: submission.problem_id,
            language: submission.language,
            source_hash: submission.source_hash,
            source_uri: submission.source_uri,
            status: SubmissionStatus::Queued,
            verdict: None,
            created_at: parse_ts("submissions", &created_at)?,
            started_at: None,
            finished_at: None,
        })
    }

    async fn submission(&self, id: SubmissionId) -> Result<Submission, StoreError> {
        let id_text = id.to_string();
        let row = sqlx::query_as!(
            SubmissionRecord,
            "SELECT id, user_id, problem_id, language, source_hash, source_uri,
                    status, verdict, created_at, started_at, finished_at
             FROM submissions WHERE id = ?",
            id_text,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "submissions",
            id: id_text,
        })?;

        row.into_submission()
    }

    async fn latest_verdict_for_hash(
        &self,
        user_id: UserId,
        source_hash: &str,
    ) -> Result<Option<Submission>, StoreError> {
        let user_id = user_id.to_string();
        let row = sqlx::query_as!(
            SubmissionRecord,
            "SELECT id, user_id, problem_id, language, source_hash, source_uri,
                    status, verdict, created_at, started_at, finished_at
             FROM submissions
             WHERE user_id = ? AND source_hash = ? AND verdict IS NOT NULL
             ORDER BY id DESC
             LIMIT 1",
            user_id,
            source_hash,
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(SubmissionRecord::into_submission).transpose()
    }

    async fn set_submission_status(
        &self,
        id: SubmissionId,
        status: SubmissionStatus,
    ) -> Result<(), StoreError> {
        let id_text = id.to_string();
        let mut tx = self.begin_immediate().await?;

        let row = sqlx::query!(
            "SELECT status, started_at FROM submissions WHERE id = ?",
            id_text,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "submissions",
            id: id_text.clone(),
        })?;

        let current = SubmissionStatus::parse(&row.status)
            .ok_or_else(|| corrupt("submissions", format!("bad status {:?}", row.status)))?;

        if !current.can_transition_to(status) {
            return Err(StoreError::IllegalTransition {
                id,
                from: current.to_string(),
                to: status.to_string(),
            });
        }

        let status_text = status.as_str();
        let started_at = match row.started_at {
            Some(existing) => existing,
            None => now(),
        };

        sqlx::query!(
            "UPDATE submissions SET status = ?, started_at = ? WHERE id = ?",
            status_text,
            started_at,
            id_text,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn finish_submission(
        &self,
        id: SubmissionId,
        verdict: Verdict,
    ) -> Result<(), StoreError> {
        let id_text = id.to_string();
        let mut tx = self.begin_immediate().await?;

        let row = sqlx::query!(
            "SELECT user_id, problem_id, status FROM submissions WHERE id = ?",
            id_text,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "submissions",
            id: id_text.clone(),
        })?;

        let current = SubmissionStatus::parse(&row.status)
            .ok_or_else(|| corrupt("submissions", format!("bad status {:?}", row.status)))?;
        if !current.can_transition_to(SubmissionStatus::Done) {
            return Err(StoreError::IllegalTransition {
                id,
                from: current.to_string(),
                to: SubmissionStatus::Done.to_string(),
            });
        }

        let verdict_text = verdict.as_str();
        let finished_at = now();
        sqlx::query!(
            "UPDATE submissions
             SET status = 'DONE', verdict = ?, finished_at = ?
             WHERE id = ?",
            verdict_text,
            finished_at,
            id_text,
        )
        .execute(&mut *tx)
        .await?;

        // Progress bookkeeping in the same transaction: attempts always
        // count; solved_at is stamped once, by the first AC, and only for
        // verdicts that reflect the learner's code. ANSI-plain upsert:
        // UPDATE first, INSERT if nothing matched.
        let counts = verdict.counts_against_user() || verdict.is_accepted();
        if counts {
            let solved_at = verdict.is_accepted().then(|| finished_at.clone());
            let updated = sqlx::query!(
                "UPDATE user_progress
                 SET attempts = attempts + 1,
                     solved_at = CASE
                         WHEN solved_at IS NULL THEN ?
                         ELSE solved_at
                     END
                 WHERE user_id = ? AND problem_id = ?",
                solved_at,
                row.user_id,
                row.problem_id,
            )
            .execute(&mut *tx)
            .await?;

            if updated.rows_affected() == 0 {
                sqlx::query!(
                    "INSERT INTO user_progress (user_id, problem_id, solved_at, attempts)
                     VALUES (?, ?, ?, 1)",
                    row.user_id,
                    row.problem_id,
                    solved_at,
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    async fn submissions_for_user(
        &self,
        user_id: UserId,
        limit: u32,
    ) -> Result<Vec<Submission>, StoreError> {
        let user_id = user_id.to_string();
        let limit = i64::from(limit);
        let rows = sqlx::query_as!(
            SubmissionRecord,
            "SELECT id, user_id, problem_id, language, source_hash, source_uri,
                    status, verdict, created_at, started_at, finished_at
             FROM submissions
             WHERE user_id = ?
             ORDER BY id DESC
             LIMIT ?",
            user_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(SubmissionRecord::into_submission)
            .collect()
    }

    async fn in_flight_submissions(&self) -> Result<Vec<Submission>, StoreError> {
        let rows = sqlx::query_as!(
            SubmissionRecord,
            "SELECT id, user_id, problem_id, language, source_hash, source_uri,
                    status, verdict, created_at, started_at, finished_at
             FROM submissions
             WHERE status <> 'DONE'
             ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(SubmissionRecord::into_submission)
            .collect()
    }

    async fn record_run(&self, run: NewRun) -> Result<Run, StoreError> {
        let id = RunId::generate();
        let id_text = id.to_string();
        let submission_id = run.submission_id.to_string();
        let attempt = i64::from(run.attempt);
        let verdict_text = run.verdict.map(Verdict::as_str);
        let boot_ms = run.boot_ms.map(i64::from);
        let build_ms = run.build_ms.map(i64::from);
        let run_ms = run.run_ms.map(i64::from);
        let created_at = now();

        let result = sqlx::query!(
            "INSERT INTO runs
                 (id, submission_id, attempt, runner_id, fixture, exit_code,
                  verdict, output, log_uri, boot_ms, build_ms, run_ms, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            id_text,
            submission_id,
            attempt,
            run.runner_id,
            run.fixture,
            run.exit_code,
            verdict_text,
            run.output,
            run.log_uri,
            boot_ms,
            build_ms,
            run_ms,
            created_at,
        )
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(Run {
                id,
                submission_id: run.submission_id,
                attempt: run.attempt,
                runner_id: run.runner_id,
                fixture: run.fixture,
                exit_code: run.exit_code,
                verdict: run.verdict,
                output: run.output,
                log_uri: run.log_uri,
                boot_ms: run.boot_ms,
                build_ms: run.build_ms,
                run_ms: run.run_ms,
                created_at: parse_ts("runs", &created_at)?,
            }),
            Err(e) if is_unique_violation(&e) => Err(StoreError::Conflict {
                entity: "runs",
                detail: format!(
                    "attempt {} for submission {} already recorded",
                    run.attempt, run.submission_id
                ),
            }),
            Err(e) => Err(e.into()),
        }
    }

    async fn runs_for_submission(&self, id: SubmissionId) -> Result<Vec<Run>, StoreError> {
        const E: &str = "runs";
        let id_text = id.to_string();
        let rows = sqlx::query!(
            "SELECT id, submission_id, attempt, runner_id, fixture, exit_code,
                    verdict, output, log_uri, boot_ms, build_ms, run_ms, created_at
             FROM runs
             WHERE submission_id = ?
             ORDER BY attempt",
            id_text,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Run {
                    id: row
                        .id
                        .parse()
                        .map_err(|_| corrupt(E, format!("bad id {:?}", row.id)))?,
                    submission_id: id,
                    attempt: u32::try_from(row.attempt)
                        .map_err(|_| corrupt(E, format!("bad attempt {}", row.attempt)))?,
                    runner_id: row.runner_id,
                    fixture: row.fixture,
                    exit_code: row
                        .exit_code
                        .map(i32::try_from)
                        .transpose()
                        .map_err(|_| corrupt(E, "exit_code out of i32 range"))?,
                    verdict: row
                        .verdict
                        .as_deref()
                        .map(|v| {
                            Verdict::parse(v)
                                .ok_or_else(|| corrupt(E, format!("bad verdict {v:?}")))
                        })
                        .transpose()?,
                    output: row.output,
                    log_uri: row.log_uri,
                    boot_ms: row
                        .boot_ms
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|_| corrupt(E, "boot_ms out of range"))?,
                    build_ms: row
                        .build_ms
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|_| corrupt(E, "build_ms out of range"))?,
                    run_ms: row
                        .run_ms
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|_| corrupt(E, "run_ms out of range"))?,
                    created_at: parse_ts(E, &row.created_at)?,
                })
            })
            .collect()
    }

    async fn progress(
        &self,
        user_id: UserId,
        problem_id: &ProblemId,
    ) -> Result<Option<UserProgress>, StoreError> {
        const E: &str = "user_progress";
        let user_id_text = user_id.to_string();
        let problem_id_text = problem_id.as_str();
        let row = sqlx::query!(
            "SELECT solved_at, attempts
             FROM user_progress
             WHERE user_id = ? AND problem_id = ?",
            user_id_text,
            problem_id_text,
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(UserProgress {
                user_id,
                problem_id: problem_id.clone(),
                solved_at: parse_opt_ts(E, row.solved_at.as_deref())?,
                attempts: u32::try_from(row.attempts)
                    .map_err(|_| corrupt(E, format!("bad attempts {}", row.attempts)))?,
            })
        })
        .transpose()
    }
}
