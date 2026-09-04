//! Boot-time content indexing.
//!
//! Walk `content/`, validate everything, build the immutable in-memory index
//! the API serves from. Any invalid problem fails the whole boot with a
//! report naming every violation: a judge serving a broken problem is worse
//! than a judge that will not start, and a typo silently removing a problem
//! from the catalog is worse than either.
//!
//! The index never mutates after boot (no hot reload in the MVP), so the API
//! holds it behind a plain `Arc` with no lock.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use satunera_contracts::{
    load_config, load_problem, Config, DirSource, Language, Problem, ProblemId, ProblemSource,
    CONFIG_MANIFEST, PROBLEM_MANIFEST, STATEMENT_FILE,
};
use satunera_store::ProblemRow;

/// Everything the API needs for one problem, computed once at boot.
#[derive(Debug)]
pub struct LoadedProblem {
    pub problem: Problem,
    /// The README, rendered and sanitized.
    pub statement_html: String,
    /// One `.tar.gz` of `srcs/<lang>` per declared language. Small and
    /// immutable between restarts, so they live in memory.
    pub scaffolds: BTreeMap<Language, Vec<u8>>,
    /// Strong ETag over the manifest and the rendered statement.
    pub etag: String,
}

/// The immutable index. Built by [`ProblemIndex::load`], shared as
/// `Arc<ProblemIndex>`.
#[derive(Debug)]
pub struct ProblemIndex {
    pub config: Config,
    pub commit_sha: String,
    problems: BTreeMap<ProblemId, LoadedProblem>,
}

/// Everything wrong with the content repo, reported at once.
#[derive(Debug, thiserror::Error)]
pub struct BootError {
    pub problems: Vec<String>,
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "the content repo is not servable ({} violation{}):",
            self.problems.len(),
            if self.problems.len() == 1 { "" } else { "s" }
        )?;
        for problem in &self.problems {
            writeln!(f, "  - {problem}")?;
        }
        Ok(())
    }
}

impl ProblemIndex {
    /// Walk the content directory and build the index, or refuse with the
    /// full list of violations.
    pub fn load(content_dir: &Path) -> Result<Self, BootError> {
        let mut violations: Vec<String> = Vec::new();

        let config = match std::fs::read(content_dir.join(CONFIG_MANIFEST)) {
            Ok(bytes) => match load_config(&bytes) {
                Ok(config) => Some(config),
                Err(e) => {
                    violations.push(format!("{CONFIG_MANIFEST}: {e}"));
                    None
                }
            },
            Err(e) => {
                violations.push(format!("{CONFIG_MANIFEST}: could not be read: {e}"));
                None
            }
        };

        let mut problems: BTreeMap<ProblemId, LoadedProblem> = BTreeMap::new();
        for dir in problem_dirs(content_dir, &mut violations) {
            let dir_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match load_one(&dir, &dir_name, config.as_ref()) {
                Ok(loaded) => {
                    problems.insert(loaded.problem.id.clone(), loaded);
                }
                Err(mut errs) => violations.append(&mut errs),
            }
        }

        // Cross-file: every problem a track references must exist.
        if let Some(config) = &config {
            for id in config.referenced_problems() {
                if !problems.contains_key(id) {
                    violations.push(format!(
                        "{CONFIG_MANIFEST}: track references problem {id:?} but no such \
                         directory carries a {PROBLEM_MANIFEST}"
                    ));
                }
            }
        }

        if !violations.is_empty() {
            return Err(BootError {
                problems: violations,
            });
        }
        let config = config.expect("no violations implies config loaded");

        Ok(Self {
            config,
            commit_sha: commit_sha(content_dir),
            problems,
        })
    }

    pub fn get(&self, id: &ProblemId) -> Option<&LoadedProblem> {
        self.problems.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoadedProblem> {
        self.problems.values()
    }

    pub fn len(&self) -> usize {
        self.problems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.problems.is_empty()
    }

    /// The rows T06's `replace_problems` materializes at boot.
    pub fn problem_rows(&self) -> Vec<ProblemRow> {
        self.problems
            .values()
            .map(|loaded| ProblemRow {
                id: loaded.problem.id.clone(),
                commit_sha: self.commit_sha.clone(),
                title: loaded.problem.title.clone(),
                category: loaded.problem.category.as_str().to_string(),
                difficulty: loaded.problem.difficulty.as_str().to_string(),
                manifest: serde_json::to_string(&loaded.problem)
                    .expect("a loaded problem serializes"),
                updated_at: time::OffsetDateTime::now_utc(),
            })
            .collect()
    }
}

/// Directories under the content root carrying a `problem.json`.
fn problem_dirs(content_dir: &Path, violations: &mut Vec<String>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(content_dir) {
        Ok(entries) => entries,
        Err(e) => {
            violations.push(format!(
                "content directory {}: could not be read: {e}",
                content_dir.display()
            ));
            return Vec::new();
        }
    };

    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir() && path.join(PROBLEM_MANIFEST).is_file())
        .collect();
    dirs.sort();
    dirs
}

fn load_one(
    dir: &Path,
    dir_name: &str,
    config: Option<&Config>,
) -> Result<LoadedProblem, Vec<String>> {
    let mut violations: Vec<String> = Vec::new();
    let label = |path: &str| format!("{dir_name}/{path}");

    let source = match DirSource::new(dir) {
        Ok(source) => source,
        Err(e) => return Err(vec![format!("{dir_name}: {e}")]),
    };

    let manifest = match source.read(PROBLEM_MANIFEST) {
        Ok(bytes) => bytes,
        Err(e) => return Err(vec![format!("{}: {e}", label(PROBLEM_MANIFEST))]),
    };
    let problem = match load_problem(&manifest) {
        Ok(problem) => problem,
        Err(e) => return Err(vec![format!("{}: {e}", label(PROBLEM_MANIFEST))]),
    };

    // Cross-file rules the schema cannot see (ADR 0001 assigns them here).
    if problem.id.as_str() != dir_name {
        violations.push(format!(
            "{}: id is {:?} but the directory is named {:?}; they must match",
            label(PROBLEM_MANIFEST),
            problem.id.as_str(),
            dir_name,
        ));
    }
    if let Some(config) = config {
        if !config.has_category(&problem.category) {
            violations.push(format!(
                "{}: category {:?} is not declared in {CONFIG_MANIFEST}",
                label(PROBLEM_MANIFEST),
                problem.category.as_str(),
            ));
        }
    }

    let statement_html = match source.read(STATEMENT_FILE) {
        Ok(bytes) => {
            let markdown = String::from_utf8_lossy(&bytes);
            if crate::render::is_valid_statement(&markdown) {
                crate::render::render_statement(&markdown)
            } else {
                violations.push(format!(
                    "{}: statement must be non-empty and carry at least one heading",
                    label(STATEMENT_FILE),
                ));
                String::new()
            }
        }
        Err(e) => {
            violations.push(format!("{}: {e}", label(STATEMENT_FILE)));
            String::new()
        }
    };

    let mut scaffolds = BTreeMap::new();
    for language in problem.languages.iter().copied() {
        let prefix = problem.scaffold_dir(language);
        match scaffold_tarball(&source, &prefix) {
            Ok(tarball) => {
                scaffolds.insert(language, tarball);
            }
            Err(e) => violations.push(format!("{}: {e}", label(&prefix))),
        }
    }

    if !violations.is_empty() {
        return Err(violations);
    }

    let etag = {
        let mut hasher = Sha256::new();
        hasher.update(&manifest);
        hasher.update(statement_html.as_bytes());
        format!("\"{}\"", hex::encode(hasher.finalize()))
    };

    Ok(LoadedProblem {
        problem,
        statement_html,
        scaffolds,
        etag,
    })
}

#[derive(Debug, thiserror::Error)]
enum ScaffoldError {
    #[error("scaffold directory is missing or empty; every declared language needs one")]
    Empty,
    #[error("{0}")]
    Source(#[from] satunera_contracts::SourceError),
    #[error("could not build the tarball: {0}")]
    Io(#[from] std::io::Error),
}

/// A deterministic `.tar.gz` of one scaffold directory, entry paths relative
/// to the scaffold root so it extracts straight into a buildable project.
fn scaffold_tarball(source: &DirSource, prefix: &str) -> Result<Vec<u8>, ScaffoldError> {
    let files = source.list(prefix)?;
    if files.is_empty() {
        return Err(ScaffoldError::Empty);
    }

    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar = tar::Builder::new(gz);
    let strip = format!("{prefix}/");

    for path in &files {
        let data = source.read(path)?;
        let executable = source.is_executable(path)?;
        let relative = path.strip_prefix(&strip).unwrap_or(path);

        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(if executable { 0o755 } else { 0o644 });
        // Fixed mtime keeps the tarball byte-identical across boots, which
        // keeps the ETag stable.
        header.set_mtime(0);
        header.set_cksum();
        tar.append_data(&mut header, relative, data.as_slice())?;
    }

    let gz = tar.into_inner()?;
    Ok(gz.finish()?)
}

/// The content repo's commit, recorded next to the materialized rows. Not
/// every deployment serves content from a git checkout, so failure is a
/// value, not an error.
fn commit_sha(content_dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(content_dir)
        .args(["rev-parse", "HEAD"])
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => "unversioned".to_string(),
    }
}

// Tests live in tests/, where a whole content tree is easier to fixture.
