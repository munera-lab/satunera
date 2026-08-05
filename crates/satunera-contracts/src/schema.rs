//! Schema validation, run before deserialization.
//!
//! Serde rejects the wrong shape but explains it to a Rust developer
//! ("invalid type: string, expected u32 at line 4 column 18"). The person
//! reading these messages authors Bitcoin exercises, so the schema validator
//! runs first and reports every violation at once with a pointer to the value.

use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::Value;

use crate::error::{ContractFile, SchemaViolation};

/// The frozen schemas, compiled into the binary so `judgectl` and the server
/// cannot disagree about which version they are enforcing.
pub const CONFIG_SCHEMA: &str = include_str!("../../../schemas/config.schema.json");
pub const PROBLEM_SCHEMA: &str = include_str!("../../../schemas/problem.schema.json");
pub const VERDICT_SCHEMA: &str = include_str!("../../../schemas/verdict.schema.json");

fn compile(source: &str, name: &str) -> Validator {
    let value: Value =
        serde_json::from_str(source).unwrap_or_else(|e| panic!("{name} is not valid JSON: {e}"));
    jsonschema::validator_for(&value)
        .unwrap_or_else(|e| panic!("{name} is not a valid schema: {e}"))
}

fn config_validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| compile(CONFIG_SCHEMA, "config.schema.json"))
}

fn problem_validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| compile(PROBLEM_SCHEMA, "problem.schema.json"))
}

fn verdict_validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| compile(VERDICT_SCHEMA, "verdict.schema.json"))
}

fn validator_for(file: ContractFile) -> &'static Validator {
    match file {
        ContractFile::Config => config_validator(),
        ContractFile::Problem => problem_validator(),
    }
}

/// Check a parsed document against its schema, collecting every violation.
///
/// All of them, not the first: an author fixing one field per run is a bad
/// afternoon.
pub fn validate(file: ContractFile, document: &Value) -> Vec<SchemaViolation> {
    collect(validator_for(file), document)
}

/// Check a verdict object against `verdict.schema.json`.
///
/// This is the strict, authoring-time check `judgectl` runs. The runtime parser
/// in [`crate::verdict`] is deliberately more forgiving.
pub fn validate_verdict(document: &Value) -> Vec<SchemaViolation> {
    collect(verdict_validator(), document)
}

fn collect(validator: &Validator, document: &Value) -> Vec<SchemaViolation> {
    let mut violations: Vec<SchemaViolation> = validator
        .iter_errors(document)
        .map(|error| SchemaViolation {
            pointer: error.instance_path().to_string(),
            message: error.to_string(),
        })
        .collect();

    // Validator output order is not guaranteed. Sorting keeps CI diffs and
    // golden-fixture assertions stable.
    violations.sort_by(|a, b| {
        a.pointer
            .cmp(&b.pointer)
            .then_with(|| a.message.cmp(&b.message))
    });
    violations.dedup();
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_schemas_compile() {
        config_validator();
        problem_validator();
        verdict_validator();
    }

    #[test]
    fn violations_point_at_the_offending_value() {
        let document = serde_json::json!({
            "schema_version": 1,
            "id": "week-1",
            "title": "Spend a P2WPKH output",
            "category": "transactions",
            "difficulty": "easy",
            "languages": ["rust", "go"],
            "fixture": "funded-p2wpkh"
        });

        let violations = validate(ContractFile::Problem, &document);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].pointer, "/languages/1");
    }
}
