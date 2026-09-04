//! Boot and API tests against a content tree built in a tempdir.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use satunera_server::{api, ProblemIndex};

// -- fixture -----------------------------------------------------------

const CONFIG: &str = r#"{
  "schema_version": 1,
  "title": "Test Judge",
  "categories": [
    { "id": "transactions", "title": "Transactions" },
    { "id": "psbt", "title": "PSBT" }
  ],
  "tracks": [
    { "id": "core", "title": "Core", "problems": ["week-1", "week-2"] }
  ]
}"#;

fn manifest(id: &str, category: &str, difficulty: &str) -> String {
    format!(
        r#"{{
  "schema_version": 1,
  "id": "{id}",
  "title": "Problem {id}",
  "category": "{category}",
  "difficulty": "{difficulty}",
  "languages": ["rust", "py"],
  "fixture": "funded-p2wpkh"
}}"#
    )
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn write_problem(root: &Path, id: &str, category: &str, difficulty: &str, statement: &str) {
    let dir = root.join(id);
    write(
        &dir.join("problem.json"),
        &manifest(id, category, difficulty),
    );
    write(&dir.join("README.md"), statement);
    write(
        &dir.join("srcs/rust/Cargo.toml"),
        "[package]\nname = \"solution\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&dir.join("srcs/rust/src/main.rs"), "fn main() {}\n");
    write(&dir.join("srcs/py/main.py"), "print(\"hi\")\n");
}

/// A valid two-problem content tree.
fn valid_content() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("config.json"), CONFIG);
    write_problem(
        dir.path(),
        "week-1",
        "transactions",
        "easy",
        "# Spend a P2WPKH output\n\nUse `bitcoin-cli`.\n",
    );
    write_problem(
        dir.path(),
        "week-2",
        "psbt",
        "medium",
        "# Build a PSBT\n\nBody.\n",
    );
    dir
}

fn app(index: ProblemIndex) -> axum::Router {
    api::router(Arc::new(index))
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body.to_vec())
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap()
}

// -- boot --------------------------------------------------------------

#[test]
fn a_valid_tree_boots() {
    let dir = valid_content();
    let index = ProblemIndex::load(dir.path()).unwrap();
    assert_eq!(index.len(), 2);
}

#[test]
fn an_invalid_problem_refuses_the_whole_boot() {
    let dir = valid_content();
    // week-3: category not declared in config.json, and no heading.
    write_problem(dir.path(), "week-3", "nonexistent", "easy", "no heading\n");

    let err = ProblemIndex::load(dir.path()).unwrap_err();
    let report = err.to_string();
    assert!(report.contains("week-3"), "{report}");
    assert!(report.contains("category"), "{report}");
    assert!(report.contains("heading"), "{report}");
}

#[test]
fn id_must_match_directory_name() {
    let dir = valid_content();
    let path = dir.path().join("week-9/problem.json");
    write(&path, &manifest("week-8", "psbt", "hard"));
    write(&dir.path().join("week-9/README.md"), "# T\n\nBody\n");
    write(
        &dir.path().join("week-9/srcs/rust/main.rs"),
        "fn main(){}\n",
    );
    write(&dir.path().join("week-9/srcs/py/main.py"), "x=1\n");

    let report = ProblemIndex::load(dir.path()).unwrap_err().to_string();
    assert!(
        report.contains("week-8") && report.contains("week-9"),
        "{report}"
    );
}

#[test]
fn a_track_referencing_a_missing_problem_refuses_boot() {
    let dir = valid_content();
    std::fs::remove_dir_all(dir.path().join("week-2")).unwrap();

    let report = ProblemIndex::load(dir.path()).unwrap_err().to_string();
    assert!(report.contains("week-2"), "{report}");
    assert!(report.contains("track"), "{report}");
}

#[test]
fn a_missing_scaffold_language_refuses_boot() {
    let dir = valid_content();
    std::fs::remove_dir_all(dir.path().join("week-1/srcs/py")).unwrap();

    let report = ProblemIndex::load(dir.path()).unwrap_err().to_string();
    assert!(report.contains("srcs/py"), "{report}");
}

// -- statements --------------------------------------------------------

#[test]
fn statement_xss_is_stripped() {
    let dir = valid_content();
    write(
        &dir.path().join("week-1/README.md"),
        "# Title\n\n<script>alert(1)</script>\n\n<img src=x onerror=alert(2)>\n\nBody.\n",
    );

    let index = ProblemIndex::load(dir.path()).unwrap();
    let html = &index
        .get(&satunera_contracts::ProblemId::new("week-1"))
        .unwrap()
        .statement_html;
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("onerror"), "{html}");
    assert!(html.contains("Body."));
}

// -- API ---------------------------------------------------------------

#[tokio::test]
async fn catalog_serves_config() {
    let dir = valid_content();
    let router = app(ProblemIndex::load(dir.path()).unwrap());

    let (status, _, body) = get(&router, "/api/v1/catalog").await;
    assert_eq!(status, StatusCode::OK);
    let value = json(&body);
    assert_eq!(value["title"], "Test Judge");
    assert_eq!(value["categories"].as_array().unwrap().len(), 2);
    assert_eq!(value["tracks"][0]["problems"][0], "week-1");
}

#[tokio::test]
async fn list_omits_statement_bodies_and_filters() {
    let dir = valid_content();
    let router = app(ProblemIndex::load(dir.path()).unwrap());

    let (status, _, body) = get(&router, "/api/v1/problems").await;
    assert_eq!(status, StatusCode::OK);
    let list = json(&body);
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(list[0].get("statement_html").is_none());

    let (_, _, body) = get(&router, "/api/v1/problems?category=psbt").await;
    let filtered = json(&body);
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["id"], "week-2");

    let (_, _, body) = get(&router, "/api/v1/problems?difficulty=easy").await;
    assert_eq!(json(&body).as_array().unwrap().len(), 1);

    let (_, _, body) = get(&router, "/api/v1/problems?category=psbt&difficulty=easy").await;
    assert_eq!(json(&body).as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn detail_serves_statement_with_etag_and_304() {
    let dir = valid_content();
    let router = app(ProblemIndex::load(dir.path()).unwrap());

    let (status, headers, body) = get(&router, "/api/v1/problems/week-1").await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let value = json(&body);
    assert_eq!(value["id"], "week-1");
    assert!(value["statement_html"].as_str().unwrap().contains("<h1>"));

    let response = router
        .clone()
        .oneshot(
            Request::get("/api/v1/problems/week-1")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn unknown_problem_is_a_json_404() {
    let dir = valid_content();
    let router = app(ProblemIndex::load(dir.path()).unwrap());

    let (status, _, body) = get(&router, "/api/v1/problems/week-99").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json(&body)["error"].as_str().unwrap().contains("week-99"));
}

#[tokio::test]
async fn scaffold_tarball_extracts_to_the_project() {
    let dir = valid_content();
    let router = app(ProblemIndex::load(dir.path()).unwrap());

    let (status, headers, body) = get(&router, "/api/v1/problems/week-1/scaffold/rust").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/gzip"
    );

    let gz = flate2::read::GzDecoder::new(body.as_slice());
    let mut archive = tar::Archive::new(gz);
    let entries: Vec<String> = archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().display().to_string())
        .collect();
    assert!(entries.contains(&"Cargo.toml".to_string()), "{entries:?}");
    assert!(entries.contains(&"src/main.rs".to_string()), "{entries:?}");

    // Unknown language and undeclared language are 404s.
    let (status, _, _) = get(&router, "/api/v1/problems/week-1/scaffold/go").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get(&router, "/api/v1/problems/week-1/scaffold/cpp").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// -- store materialization ---------------------------------------------

#[tokio::test]
async fn boot_materializes_the_problems_table() {
    let dir = valid_content();
    let store = satunera_store::SqliteStore::in_memory().await.unwrap();

    let index = satunera_server::boot(dir.path(), &store).await.unwrap();
    assert_eq!(index.len(), 2);

    use satunera_store::Store;
    let rows = store.problems().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id.as_str(), "week-1");
    // The manifest round-trips through the row.
    let manifest: satunera_contracts::Problem = serde_json::from_str(&rows[0].manifest).unwrap();
    assert_eq!(manifest.title, "Problem week-1");
}
