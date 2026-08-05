//! The golden corpus: every sample in `testdata/golden/` must classify the way
//! its name and sidecar say it does.
//!
//! The validator (T02), the problem API (T07), and the Author Studio (T10) all
//! test against these same files, so a schema change that breaks one of them
//! breaks here first.

use std::fs;
use std::path::{Path, PathBuf};

use satunera_contracts::error::ContractFile;
use satunera_contracts::{load_config, load_problem, schema, ContractError, ValidationOutput};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/golden")
        .canonicalize()
        .expect("testdata/golden must exist at the workspace root")
}

/// Fixture files, excluding the `.expect.json` sidecars that annotate them.
fn fixtures(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && !path.to_string_lossy().ends_with(".expect.json")
        })
        .collect();
    paths.sort();
    paths
}

/// Which schema a fixture is written against. Encoded in the file name so a new
/// sample needs no registration step.
fn file_of(path: &Path) -> ContractFile {
    let name = path.file_name().unwrap().to_string_lossy();
    if name.starts_with("config-") {
        ContractFile::Config
    } else {
        ContractFile::Problem
    }
}

fn load(file: ContractFile, bytes: &[u8]) -> Result<serde_json::Value, ContractError> {
    match file {
        ContractFile::Config => load_config(bytes).map(|c| serde_json::to_value(c).unwrap()),
        ContractFile::Problem => load_problem(bytes).map(|p| serde_json::to_value(p).unwrap()),
    }
}

#[derive(serde::Deserialize)]
struct Expectation {
    /// `problem` or `config`.
    file: String,
    /// JSON Pointer the violation must name. Empty means the document root.
    pointer: String,
    /// Why this sample is rejected, in the words an author would use.
    reason: String,
}

#[test]
fn every_valid_fixture_loads() {
    let dir = golden_dir().join("valid");
    let paths = fixtures(&dir);
    assert!(
        paths.len() >= 6,
        "the corpus must hold at least 6 valid samples, found {}",
        paths.len()
    );

    for path in paths {
        let bytes = fs::read(&path).unwrap();
        if let Err(err) = load(file_of(&path), &bytes) {
            panic!(
                "{} should be valid but was rejected:\n{err}",
                path.display()
            );
        }
    }
}

/// Round-trip in both directions: a loaded document, serialized again, must
/// still satisfy the schema. This is what catches a serde attribute and a
/// schema property drifting apart.
#[test]
fn valid_fixtures_round_trip_back_through_the_schema() {
    for path in fixtures(&golden_dir().join("valid")) {
        let file = file_of(&path);
        let bytes = fs::read(&path).unwrap();
        let reserialized = load(file, &bytes).unwrap();

        let violations = schema::validate(file, &reserialized);
        assert!(
            violations.is_empty(),
            "{} does not survive a round trip: {violations:?}",
            path.display()
        );
    }
}

#[test]
fn every_invalid_fixture_is_rejected_for_the_documented_reason() {
    let dir = golden_dir().join("invalid");
    let paths = fixtures(&dir);
    assert!(
        paths.len() >= 10,
        "the corpus must hold at least 10 invalid samples, found {}",
        paths.len()
    );

    for path in paths {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let sidecar = dir.join(format!("{stem}.expect.json"));
        let expectation: Expectation =
            serde_json::from_slice(&fs::read(&sidecar).unwrap_or_else(|e| {
                panic!(
                    "{}: every invalid sample needs a sidecar annotating the rule it breaks: {e}",
                    sidecar.display()
                )
            }))
            .unwrap();

        assert!(
            !expectation.reason.trim().is_empty(),
            "{}: the sidecar must say why",
            sidecar.display()
        );

        let file = match expectation.file.as_str() {
            "config" => ContractFile::Config,
            "problem" => ContractFile::Problem,
            other => panic!("{}: unknown file kind {other}", sidecar.display()),
        };
        assert_eq!(
            file,
            file_of(&path),
            "{}: file name and sidecar disagree about which schema applies",
            path.display()
        );

        let bytes = fs::read(&path).unwrap();
        let err = match load(file, &bytes) {
            Ok(_) => panic!(
                "{} was accepted but should be rejected: {}",
                path.display(),
                expectation.reason
            ),
            Err(err) => err,
        };

        let pointers: Vec<&str> = err
            .violations()
            .iter()
            .map(|v| v.pointer.as_str())
            .collect();
        assert!(
            pointers.contains(&expectation.pointer.as_str()),
            "{} was rejected but not at {:?}: got {pointers:?}\n{err}",
            path.display(),
            expectation.pointer
        );
    }
}

/// The error an author reads has to name the file and point at the value. This
/// asserts the shape of that message, because the message is the product.
#[test]
fn rejection_messages_name_the_file_and_the_field() {
    let path = golden_dir().join("invalid/unknown-language.json");
    let err = load_problem(&fs::read(path).unwrap()).unwrap_err();
    let rendered = err.to_string();

    assert!(rendered.starts_with("problem.json:"), "{rendered}");
    assert!(rendered.contains("/languages/1"), "{rendered}");
}

#[test]
fn verdict_fixtures_classify_correctly() {
    let dir = golden_dir().join("verdict");

    for path in fixtures(&dir.join("valid")) {
        let bytes = fs::read(&path).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let violations = schema::validate_verdict(&document);
        assert!(
            violations.is_empty(),
            "{} should satisfy verdict.schema.json: {violations:?}",
            path.display()
        );

        ValidationOutput::from_line(&String::from_utf8(bytes).unwrap())
            .unwrap_or_else(|e| panic!("{} should parse at run time: {e}", path.display()));
    }

    for path in fixtures(&dir.join("invalid")) {
        let bytes = fs::read(&path).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(
            !schema::validate_verdict(&document).is_empty(),
            "{} should be rejected by verdict.schema.json",
            path.display()
        );
    }
}

/// The strict schema and the forgiving runtime parser disagree on purpose, and
/// that disagreement is a documented decision rather than an oversight.
#[test]
fn an_unknown_verdict_key_fails_authoring_but_survives_a_run() {
    let bytes = fs::read(golden_dir().join("verdict/invalid/unknown-key.json")).unwrap();
    let line = String::from_utf8(bytes).unwrap();

    let document: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(!schema::validate_verdict(&document).is_empty());

    let output = ValidationOutput::from_line(&line).expect("a stray key must not cost a run");
    assert_eq!(output.unknown_keys, vec!["msg"]);
}
