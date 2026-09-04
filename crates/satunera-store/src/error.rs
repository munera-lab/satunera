//! Error types for the two storage seams.

use crate::id::SubmissionId;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The row does not exist. Callers that can answer 404 branch on this.
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    /// A uniqueness rule was violated: duplicate handle, duplicate email,
    /// or a second run claiming the same attempt number.
    #[error("conflict on {entity}: {detail}")]
    Conflict {
        entity: &'static str,
        detail: String,
    },

    /// A status transition the lifecycle does not allow, e.g. finishing a
    /// submission that is already terminal.
    #[error("submission {id}: illegal transition {from} -> {to}")]
    IllegalTransition {
        id: SubmissionId,
        from: String,
        to: String,
    },

    /// A row read back from the database failed to decode: a bad ULID, an
    /// unknown status, a timestamp that is not RFC 3339. Always a bug or a
    /// hand-edited database, never user input.
    #[error("corrupt row in {entity}: {detail}")]
    CorruptRow {
        entity: &'static str,
        detail: String,
    },

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
}

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob not found: {key}")]
    NotFound { key: String },

    #[error("blob key escapes the store root: {key}")]
    InvalidKey { key: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
