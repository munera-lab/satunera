use std::fmt;

/// Which frozen file a diagnostic is about. Kept as an enum so error messages
/// always name a file the author can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFile {
    Config,
    Problem,
}

impl ContractFile {
    pub const fn as_str(self) -> &'static str {
        match self {
            ContractFile::Config => "config.json",
            ContractFile::Problem => "problem.json",
        }
    }
}

impl fmt::Display for ContractFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One schema rule the document broke, addressed by a JSON Pointer into the
/// document itself. The pointer is what makes the message actionable: it names
/// the offending value rather than the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    /// JSON Pointer to the failing value, e.g. `/languages/2`. Empty for the root.
    pub pointer: String,
    /// The validator's message, written for the author.
    pub message: String,
}

impl fmt::Display for SchemaViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pointer.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.pointer, self.message)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("{file}: not valid JSON: {source}")]
    Json {
        file: ContractFile,
        #[source]
        source: serde_json::Error,
    },

    #[error("{file}: {}\n{}", plural(.violations.len()), indent(.violations))]
    Schema {
        file: ContractFile,
        violations: Vec<SchemaViolation>,
    },

    /// The document passed the schema but the typed model rejected it. This is
    /// a bug in the schema or in the types, never something an author can fix,
    /// so the message says so.
    #[error(
        "{file}: passed schema validation but could not be loaded: {source}\n\
         This is a judge bug, not a problem with your file. Please report it."
    )]
    Model {
        file: ContractFile,
        #[source]
        source: serde_json::Error,
    },

    #[error(transparent)]
    Source(#[from] crate::source::SourceError),
}

impl ContractError {
    /// The file this error is about, when it is about one.
    pub fn file(&self) -> Option<ContractFile> {
        match self {
            ContractError::Json { file, .. }
            | ContractError::Schema { file, .. }
            | ContractError::Model { file, .. } => Some(*file),
            ContractError::Source(_) => None,
        }
    }

    /// Schema violations, for callers that render their own report (T02, T10).
    pub fn violations(&self) -> &[SchemaViolation] {
        match self {
            ContractError::Schema { violations, .. } => violations,
            _ => &[],
        }
    }
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 problem".to_string()
    } else {
        format!("{n} problems")
    }
}

fn indent(violations: &[SchemaViolation]) -> String {
    violations
        .iter()
        .map(|v| format!("  {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}
