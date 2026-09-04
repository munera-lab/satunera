//! The judge's HTTP face.
//!
//! T07: walk the content repo at boot, validate every problem, build an
//! immutable in-memory index, materialize it into the `problems` table, and
//! serve the read API. Boot fails loudly if any problem is malformed; no hot
//! reload in the MVP, so a content merge means a restart.

pub mod api;
pub mod content;
pub mod render;

use std::path::Path;
use std::sync::Arc;

use satunera_store::Store;

pub use content::{BootError, LoadedProblem, ProblemIndex};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Boot(#[from] BootError),

    #[error("materializing the catalog failed: {0}")]
    Store(#[from] satunera_store::StoreError),
}

/// Load the content repo and materialize it into the store. The returned
/// index is what [`api::router`] serves from.
pub async fn boot(content_dir: &Path, store: &dyn Store) -> Result<Arc<ProblemIndex>, ServerError> {
    let index = ProblemIndex::load(content_dir)?;
    store.replace_problems(index.problem_rows()).await?;
    tracing::info!(
        problems = index.len(),
        commit = %index.commit_sha,
        "content indexed"
    );
    Ok(Arc::new(index))
}
