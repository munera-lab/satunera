//! The contract document and the code have to agree.
//!
//! The runner sets these variables and `validation.sh` reads them, and the only
//! thing keeping the two in step is `docs/validation-contract.md`. A variable
//! added in code and forgotten in the document is a variable authors never use.

use std::fs;
use std::path::{Path, PathBuf};

use satunera_contracts::{env, Verdict};

fn doc_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(name)
}

fn contract_doc() -> String {
    let path = doc_path("validation-contract.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_injected_variable_is_documented() {
    let doc = contract_doc();
    let missing: Vec<&str> = env::all()
        .into_iter()
        .filter(|name| !doc.contains(name))
        .collect();

    assert!(
        missing.is_empty(),
        "these variables are injected but absent from docs/validation-contract.md: {missing:?}"
    );
}

#[test]
fn every_documented_judge_variable_exists_in_code() {
    let doc = contract_doc();
    let known = env::all();

    // Pull `JUDGE_*` and `BITCOIN_*` tokens out of the document and check each
    // one is a constant. Catches a variable renamed in code but not in prose.
    let undeclared: Vec<String> = doc
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|token| {
            // The prose names the bare prefixes when it reserves them; those
            // are not variables.
            !env::RESERVED_PREFIXES.contains(token)
                && env::RESERVED_PREFIXES
                    .iter()
                    .any(|prefix| token.starts_with(prefix))
        })
        .filter(|token| !known.contains(token))
        .map(str::to_string)
        .collect();

    assert!(
        undeclared.is_empty(),
        "docs/validation-contract.md names variables that no constant declares: {undeclared:?}"
    );
}

#[test]
fn every_verdict_code_is_documented() {
    let doc = contract_doc();
    for verdict in Verdict::ALL {
        assert!(
            doc.contains(&format!("`{}`", verdict.as_str())),
            "{} is a verdict the runner can produce but the contract does not explain it",
            verdict
        );
    }
}

#[test]
fn the_contract_document_states_the_two_rules_that_carry_it() {
    let doc = contract_doc().to_lowercase();
    assert!(doc.contains("last non-blank line"), "verdict transport");
    assert!(doc.contains("exit code decides"), "acceptance rule");
}
