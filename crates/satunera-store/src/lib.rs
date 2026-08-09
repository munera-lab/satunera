//! Durable state for the judge, behind two seams.
//!
//! [`Store`] owns everything relational: users, the materialized problem
//! catalog, submissions, per-attempt runs, and progress. The only
//! implementation is SQLite (ADR 0002); Postgres arrives in phase 2 as a
//! second implementation passing the same suite, which is why consumers hold
//! the trait and never a pool.
//!
//! [`BlobStore`] owns bytes: source blobs content-addressed by hash, logs
//! keyed by submission. Filesystem now, S3 later, two methods either way.
//!
//! `runs` is deliberately separate from `submissions`. A runner that dies
//! mid-job and retries shows up as a second attempt for debugging, never as a
//! second row in the learner's history.

pub mod blob;
pub mod error;
pub mod id;
pub mod model;
pub mod sqlite;
pub mod store;

pub use blob::{BlobKind, BlobStore, FsBlobStore, Sha256Hash};
pub use error::{BlobError, StoreError};
pub use id::{RunId, SubmissionId, UserId};
pub use model::{
    NewRun, NewSubmission, NewUser, ProblemRow, Role, Run, Submission, SubmissionStatus, User,
    UserProgress,
};
pub use sqlite::SqliteStore;
pub use store::Store;

/// The embedded migrations. The binary carries them; no external tool needed.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
