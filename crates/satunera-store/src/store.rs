//! The relational seam.
//!
//! T07 and T08 compile against this trait, never against a pool. The SQLite
//! implementation is the MVP; the Postgres one drops in behind it in phase 2
//! and must pass the same suite.

use async_trait::async_trait;

use satunera_contracts::{ProblemId, Verdict};

use crate::error::StoreError;
use crate::id::{SubmissionId, UserId};
use crate::model::{
    NewRun, NewSubmission, NewUser, ProblemRow, Run, Submission, SubmissionStatus, User,
    UserProgress,
};

#[async_trait]
pub trait Store: Send + Sync {
    // -- users ---------------------------------------------------------

    /// Mints the id and timestamp. `Conflict` on a duplicate handle or email.
    async fn create_user(&self, user: NewUser) -> Result<User, StoreError>;

    async fn user(&self, id: UserId) -> Result<User, StoreError>;

    async fn user_by_handle(&self, handle: &str) -> Result<User, StoreError>;

    // -- problems ------------------------------------------------------

    /// Replace the materialized catalog with the given rows, atomically.
    /// T07 calls this once at boot after validating the content repo.
    async fn replace_problems(&self, problems: Vec<ProblemRow>) -> Result<(), StoreError>;

    async fn problem(&self, id: &ProblemId) -> Result<ProblemRow, StoreError>;

    async fn problems(&self) -> Result<Vec<ProblemRow>, StoreError>;

    // -- submissions ---------------------------------------------------

    /// Mints the id, stamps `created_at`, starts at `QUEUED`.
    async fn create_submission(&self, submission: NewSubmission) -> Result<Submission, StoreError>;

    async fn submission(&self, id: SubmissionId) -> Result<Submission, StoreError>;

    /// The dedupe lookup: latest submission with this hash that already has a
    /// verdict, if any. `source_hash` covers problem, language and normalized
    /// source, so one query answers "has anyone-this-user run exactly this?".
    async fn latest_verdict_for_hash(
        &self,
        user_id: UserId,
        source_hash: &str,
    ) -> Result<Option<Submission>, StoreError>;

    /// Advance the lifecycle. Stamps `started_at` on the first move out of
    /// `QUEUED`. `IllegalTransition` if the move is not allowed.
    async fn set_submission_status(
        &self,
        id: SubmissionId,
        status: SubmissionStatus,
    ) -> Result<(), StoreError>;

    /// Terminal write: status to `DONE`, verdict set, `finished_at` stamped,
    /// and `user_progress` upserted in the same transaction.
    async fn finish_submission(&self, id: SubmissionId, verdict: Verdict)
        -> Result<(), StoreError>;

    /// The learner's history, newest first.
    async fn submissions_for_user(
        &self,
        user_id: UserId,
        limit: u32,
    ) -> Result<Vec<Submission>, StoreError>;

    /// Everything not yet terminal. The recovery scan at boot: anything here
    /// after a crash is a job the process died holding.
    async fn in_flight_submissions(&self) -> Result<Vec<Submission>, StoreError>;

    // -- runs ------------------------------------------------------------

    /// Record a completed attempt. `Conflict` if the attempt number is taken.
    async fn record_run(&self, run: NewRun) -> Result<Run, StoreError>;

    async fn runs_for_submission(&self, id: SubmissionId) -> Result<Vec<Run>, StoreError>;

    // -- progress --------------------------------------------------------

    async fn progress(
        &self,
        user_id: UserId,
        problem_id: &ProblemId,
    ) -> Result<Option<UserProgress>, StoreError>;
}
