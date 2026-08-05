//! The frozen interface between problem authors and the judge.
//!
//! Three artifacts live here, and every other crate reads through them:
//!
//! - `config.json`, the instance catalog and feature flags ([`Config`]).
//! - `problem.json`, one problem's manifest ([`Problem`]).
//! - the `validation.sh` contract: the injected environment ([`env`]) and the
//!   JSON verdict on the last line of stdout ([`ValidationOutput`]).
//!
//! Plus [`ProblemSource`], the read abstraction the validator uses so it works
//! against a directory on disk and an uploaded tarball with one implementation.
//!
//! Loading is a two-step: validate against the JSON Schema, then deserialize.
//! The schema pass exists to produce messages aimed at problem authors, and it
//! reports every violation rather than stopping at the first.

pub mod config;
pub mod env;
pub mod error;
pub mod id;
pub mod problem;
pub mod schema;
pub mod source;
pub mod verdict;

pub use config::{Category, Config, Features, Track, CONFIG_SCHEMA_VERSION};
pub use error::{ContractError, ContractFile, SchemaViolation};
pub use id::{CategoryId, FixtureName, ProblemId, TrackId};
pub use problem::{
    Difficulty, Language, Limits, Problem, Services, DEFAULT_MEMORY_MB, DEFAULT_WALL_SEC,
    PROBLEM_SCHEMA_VERSION,
};
pub use source::{DirSource, ProblemSource, SourceError, TarEntry, TarSource};
pub use verdict::{
    ValidationOutput, Verdict, VerdictError, MAX_AUTOMATIC_RETRIES, MAX_MESSAGE_CHARS,
};

/// File name that marks a directory as a problem.
pub const PROBLEM_MANIFEST: &str = "problem.json";
/// File name of the instance config, at the root of the content repo.
pub const CONFIG_MANIFEST: &str = "config.json";
/// The author-supplied checker, at the root of every problem directory.
pub const VALIDATION_SCRIPT: &str = "validation.sh";
/// The problem statement, rendered to HTML at boot and served to learners.
pub const STATEMENT_FILE: &str = "README.md";

/// Parse and validate `config.json`.
pub fn load_config(bytes: &[u8]) -> Result<Config, ContractError> {
    load(ContractFile::Config, bytes)
}

/// Parse and validate `problem.json`.
pub fn load_problem(bytes: &[u8]) -> Result<Problem, ContractError> {
    load(ContractFile::Problem, bytes)
}

/// Read and validate the `problem.json` at the root of a problem source.
pub fn load_problem_from(source: &dyn ProblemSource) -> Result<Problem, ContractError> {
    let bytes = source.read(PROBLEM_MANIFEST)?;
    load_problem(&bytes)
}

fn load<T: serde::de::DeserializeOwned>(
    file: ContractFile,
    bytes: &[u8],
) -> Result<T, ContractError> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|source| ContractError::Json { file, source })?;

    let violations = schema::validate(file, &document);
    if !violations.is_empty() {
        return Err(ContractError::Schema { file, violations });
    }

    // Past the schema, a deserialization failure means the types and the schema
    // disagree, which is ours to fix, not the author's.
    serde_json::from_value(document).map_err(|source| ContractError::Model { file, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_errors_come_before_serde_errors() {
        let err = load_problem(br#"{"schema_version": 1, "id": "WEEK-1"}"#).unwrap_err();
        let message = err.to_string();
        assert!(message.starts_with("problem.json:"), "{message}");
        assert!(
            err.violations().iter().any(|v| v.pointer == "/id"),
            "{message}"
        );
    }

    #[test]
    fn invalid_json_names_the_file() {
        let err = load_config(b"{ not json }").unwrap_err();
        assert!(err.to_string().starts_with("config.json: not valid JSON"));
    }

    #[test]
    fn a_missing_manifest_reports_the_path() {
        let source = TarSource::new();
        let err = load_problem_from(&source).unwrap_err();
        assert!(err.to_string().contains("problem.json"));
    }
}
