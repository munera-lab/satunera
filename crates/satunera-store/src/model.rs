//! The rows, as Rust sees them.
//!
//! Timestamps are `time::OffsetDateTime`, stored as RFC 3339 `TEXT` and
//! parsed on read (ADR 0002: no backend-flexible column types). Statuses and
//! verdicts are enums here and `CHECK`-constrained `TEXT` in the schema, so a
//! corrupt value fails loudly on both sides of the boundary.

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use satunera_contracts::{Language, ProblemId, Verdict};

use crate::id::{RunId, SubmissionId, UserId};

/// What a user is allowed to do. Learners submit; authors also publish
/// problems; admins also manage users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Learner,
    Author,
    Admin,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Learner, Role::Author, Role::Admin];

    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Learner => "learner",
            Role::Author => "author",
            Role::Admin => "admin",
        }
    }

    pub fn parse(value: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str() == value)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a submission is in its lifecycle. `Done` is the only terminal state,
/// and the only one where `verdict` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SubmissionStatus {
    Queued,
    Preparing,
    Building,
    Booting,
    Running,
    Done,
}

impl SubmissionStatus {
    pub const ALL: [SubmissionStatus; 6] = [
        SubmissionStatus::Queued,
        SubmissionStatus::Preparing,
        SubmissionStatus::Building,
        SubmissionStatus::Booting,
        SubmissionStatus::Running,
        SubmissionStatus::Done,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            SubmissionStatus::Queued => "QUEUED",
            SubmissionStatus::Preparing => "PREPARING",
            SubmissionStatus::Building => "BUILDING",
            SubmissionStatus::Booting => "BOOTING",
            SubmissionStatus::Running => "RUNNING",
            SubmissionStatus::Done => "DONE",
        }
    }

    pub fn parse(value: &str) -> Option<SubmissionStatus> {
        SubmissionStatus::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, SubmissionStatus::Done)
    }

    /// The lifecycle only moves forward. A retry after `SE`/`IE` re-enters at
    /// `Preparing`, which is the one legal backwards edge.
    pub fn can_transition_to(self, next: SubmissionStatus) -> bool {
        use SubmissionStatus::*;
        match (self, next) {
            (Done, _) => false,
            (_, Queued) => false,
            // Retry path: a failed attempt starts over.
            (Building | Booting | Running, Preparing) => true,
            (Queued, Preparing) => true,
            (Preparing, Building | Booting | Running | Done) => true,
            (Building, Booting | Running | Done) => true,
            (Booting, Running | Done) => true,
            (Running, Done) => true,
            _ => false,
        }
    }
}

impl fmt::Display for SubmissionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub handle: String,
    pub email: String,
    /// Never serialized out of the server. The hash format is the caller's
    /// business (T08 owns auth); the store treats it as opaque.
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: Role,
    pub created_at: OffsetDateTime,
}

/// Insert form of [`User`]. The store mints the id and the timestamp.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub handle: String,
    pub email: String,
    pub password_hash: String,
    pub role: Role,
}

/// One problem, materialized from the content repo at boot (T07 writes these,
/// everything else reads). `manifest` is the full `problem.json`, stored as
/// text and parsed on demand, so the catalog page needs no filesystem walk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemRow {
    pub id: ProblemId,
    pub commit_sha: String,
    pub title: String,
    pub category: String,
    pub difficulty: String,
    pub manifest: String,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub id: SubmissionId,
    pub user_id: UserId,
    pub problem_id: ProblemId,
    pub language: Language,
    /// `sha256(problem_id + language + normalized_source)`, hex. The dedupe
    /// key: an identical resubmission returns the prior verdict.
    pub source_hash: String,
    /// Where the source blob lives, as reported by the [`crate::BlobStore`].
    pub source_uri: String,
    pub status: SubmissionStatus,
    pub verdict: Option<Verdict>,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

/// Insert form of [`Submission`]. The store mints the id, stamps
/// `created_at`, and starts the row at `QUEUED`.
#[derive(Debug, Clone)]
pub struct NewSubmission {
    pub user_id: UserId,
    pub problem_id: ProblemId,
    pub language: Language,
    pub source_hash: String,
    pub source_uri: String,
}

/// One judging attempt. Timings are what the latency dashboards read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub submission_id: SubmissionId,
    /// 1-based. Attempt 2 exists only after an `SE`/`IE` retry.
    pub attempt: u32,
    pub runner_id: String,
    pub fixture: String,
    pub exit_code: Option<i32>,
    pub verdict: Option<Verdict>,
    /// The parsed last-line JSON from `validation.sh`, re-serialized.
    pub output: Option<String>,
    /// Where the captured logs live in the blob store.
    pub log_uri: Option<String>,
    pub boot_ms: Option<u32>,
    pub build_ms: Option<u32>,
    pub run_ms: Option<u32>,
    pub created_at: OffsetDateTime,
}

/// Insert form of [`Run`], written when the attempt finishes. The runner
/// holds attempt state in memory; only completed attempts are history.
#[derive(Debug, Clone)]
pub struct NewRun {
    pub submission_id: SubmissionId,
    pub attempt: u32,
    pub runner_id: String,
    pub fixture: String,
    pub exit_code: Option<i32>,
    pub verdict: Option<Verdict>,
    pub output: Option<String>,
    pub log_uri: Option<String>,
    pub boot_ms: Option<u32>,
    pub build_ms: Option<u32>,
    pub run_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProgress {
    pub user_id: UserId,
    pub problem_id: ProblemId,
    pub solved_at: Option<OffsetDateTime>,
    pub attempts: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_is_terminal_and_queued_is_unreachable() {
        for status in SubmissionStatus::ALL {
            assert!(!SubmissionStatus::Done.can_transition_to(status));
            if status != SubmissionStatus::Queued {
                assert!(!status.can_transition_to(SubmissionStatus::Queued));
            }
        }
    }

    #[test]
    fn the_happy_path_is_legal() {
        use SubmissionStatus::*;
        let path = [Queued, Preparing, Building, Booting, Running, Done];
        for pair in path.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "{} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn retry_reenters_at_preparing() {
        use SubmissionStatus::*;
        for from in [Building, Booting, Running] {
            assert!(from.can_transition_to(Preparing));
        }
    }

    #[test]
    fn statuses_round_trip() {
        for status in SubmissionStatus::ALL {
            assert_eq!(SubmissionStatus::parse(status.as_str()), Some(status));
        }
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
    }
}
