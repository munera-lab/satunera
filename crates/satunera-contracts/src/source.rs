//! Reading a problem directory without knowing where it came from.
//!
//! The validator (T02) runs against a directory on disk from the CLI and
//! against an uploaded tarball in the Author Studio (T10). Rust has no standard
//! filesystem trait, so the abstraction lives here, next to the types both call
//! sites already depend on.
//!
//! `is_executable` is a method rather than an afterthought because the mode bit
//! on `validation.sh` is a rule the validator enforces. A VFS abstraction that
//! drops permissions cannot express that rule.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{path}: no such file in the problem directory")]
    NotFound { path: String },

    #[error("{path}: is a directory, not a file")]
    IsDirectory { path: String },

    #[error(
        "{path}: path escapes the problem directory. \
         Absolute paths, `..` segments, and symlinks pointing outside the directory are refused."
    )]
    Escapes { path: String },

    #[error("{path}: could not be read: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
}

/// A readable problem tree, rooted at the directory that holds `problem.json`.
///
/// Paths are relative, `/`-separated, and never begin with `/` or contain `..`.
/// Implementations reject anything else rather than normalizing it away.
pub trait ProblemSource {
    /// Read one file. Missing files are an error, not an empty vector: the
    /// validator distinguishes "absent" from "empty".
    fn read(&self, path: &str) -> Result<Vec<u8>, SourceError>;

    /// Every file path under `prefix`, recursively, sorted, relative to the
    /// root of the source rather than to `prefix`. An empty prefix lists the
    /// whole tree. Directories are not returned; only files are.
    fn list(&self, prefix: &str) -> Result<Vec<String>, SourceError>;

    /// Whether the file carries an executable mode bit for any of user, group,
    /// or other. Sources that genuinely cannot know return `false`.
    fn is_executable(&self, path: &str) -> Result<bool, SourceError>;
}

/// Normalize an incoming path and refuse anything that could point outside the
/// tree. Called on the entry name, before any byte is read or written, which is
/// the only place traversal can be rejected cheaply and completely.
pub fn normalize(path: &str) -> Result<String, SourceError> {
    let raw = path.replace('\\', "/");
    let trimmed = raw.trim_start_matches("./");

    if trimmed.is_empty() {
        return Err(SourceError::NotFound {
            path: path.to_string(),
        });
    }
    if trimmed.starts_with('/') || trimmed.contains(':') {
        return Err(SourceError::Escapes {
            path: path.to_string(),
        });
    }

    let mut out: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                return Err(SourceError::Escapes {
                    path: path.to_string(),
                })
            }
            other => out.push(other),
        }
    }

    if out.is_empty() {
        return Err(SourceError::NotFound {
            path: path.to_string(),
        });
    }
    Ok(out.join("/"))
}

/// A problem directory on disk. Used by `judgectl` and by the server's boot-time
/// content walk.
#[derive(Debug, Clone)]
pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    /// The root is canonicalized once so symlink escapes can be detected by
    /// prefix comparison rather than by re-resolving on every read.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SourceError> {
        let root = root.into();
        let canonical = fs::canonicalize(&root).map_err(|source| SourceError::Io {
            path: root.display().to_string(),
            source,
        })?;
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a source-relative path to a real one, refusing any result that
    /// leaves the root. A symlink to `/etc/passwd` fails here rather than being
    /// followed.
    fn resolve(&self, path: &str) -> Result<PathBuf, SourceError> {
        let rel = normalize(path)?;
        let joined = self.root.join(&rel);

        let canonical = fs::canonicalize(&joined).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => SourceError::NotFound { path: rel.clone() },
            _ => SourceError::Io {
                path: rel.clone(),
                source,
            },
        })?;

        if !canonical.starts_with(&self.root) {
            return Err(SourceError::Escapes { path: rel });
        }
        Ok(canonical)
    }

    fn walk(&self, dir: &Path, out: &mut Vec<String>) -> Result<(), SourceError> {
        let entries = fs::read_dir(dir).map_err(|source| SourceError::Io {
            path: self.relative(dir),
            source,
        })?;

        for entry in entries {
            let entry = entry.map_err(|source| SourceError::Io {
                path: self.relative(dir),
                source,
            })?;
            let path = entry.path();

            // symlink_metadata, not metadata: a symlinked directory must not be
            // descended into, or a cycle walks forever.
            let meta = fs::symlink_metadata(&path).map_err(|source| SourceError::Io {
                path: self.relative(&path),
                source,
            })?;

            if meta.is_dir() {
                self.walk(&path, out)?;
            } else {
                // Symlinks to files are listed. Reading one that escapes the
                // root fails, which is how the validator sees the violation.
                out.push(self.relative(&path));
            }
        }
        Ok(())
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl ProblemSource for DirSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, SourceError> {
        let resolved = self.resolve(path)?;
        if resolved.is_dir() {
            return Err(SourceError::IsDirectory {
                path: normalize(path)?,
            });
        }
        fs::read(&resolved).map_err(|source| SourceError::Io {
            path: normalize(path).unwrap_or_else(|_| path.to_string()),
            source,
        })
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, SourceError> {
        let start = if prefix.is_empty() || prefix == "." {
            self.root.clone()
        } else {
            match self.resolve(prefix) {
                Ok(p) => p,
                Err(SourceError::NotFound { .. }) => return Ok(Vec::new()),
                Err(e) => return Err(e),
            }
        };

        let mut out = Vec::new();
        if start.is_dir() {
            self.walk(&start, &mut out)?;
        } else {
            out.push(self.relative(&start));
        }
        out.sort();
        Ok(out)
    }

    fn is_executable(&self, path: &str) -> Result<bool, SourceError> {
        let resolved = self.resolve(path)?;
        let meta = fs::metadata(&resolved).map_err(|source| SourceError::Io {
            path: normalize(path).unwrap_or_else(|_| path.to_string()),
            source,
        })?;
        Ok(mode_is_executable(file_mode(&meta)))
    }
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    // No mode bits to read. The validator's executable-bit rule cannot be
    // enforced on such a platform, and reporting "not executable" is the
    // conservative answer.
    0
}

pub const fn mode_is_executable(mode: u32) -> bool {
    mode & 0o111 != 0
}

/// One file inside a [`TarSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarEntry {
    pub mode: u32,
    pub data: Vec<u8>,
}

/// A problem tree held in memory, built from an uploaded archive.
///
/// This type deliberately does not decompress anything. The caller expands the
/// archive and enforces its own decompressed-size cap while doing so, because
/// the cap has to apply during expansion, not after.
#[derive(Debug, Clone, Default)]
pub struct TarSource {
    entries: BTreeMap<String, TarEntry>,
}

impl TarSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one file. The archive entry name is normalized and checked here, so
    /// a `../../etc/passwd` entry is refused before its bytes are stored.
    pub fn insert(&mut self, path: &str, mode: u32, data: Vec<u8>) -> Result<(), SourceError> {
        let key = normalize(path)?;
        self.entries.insert(key, TarEntry { mode, data });
        Ok(())
    }

    pub fn from_entries<I, S>(entries: I) -> Result<Self, SourceError>
    where
        I: IntoIterator<Item = (S, u32, Vec<u8>)>,
        S: AsRef<str>,
    {
        let mut source = Self::new();
        for (path, mode, data) in entries {
            source.insert(path.as_ref(), mode, data)?;
        }
        Ok(source)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total stored bytes, for callers reporting on an upload.
    pub fn total_bytes(&self) -> usize {
        self.entries.values().map(|e| e.data.len()).sum()
    }

    fn get(&self, path: &str) -> Result<&TarEntry, SourceError> {
        let key = normalize(path)?;
        self.entries
            .get(&key)
            .ok_or(SourceError::NotFound { path: key })
    }
}

impl ProblemSource for TarSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, SourceError> {
        Ok(self.get(path)?.data.clone())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, SourceError> {
        if prefix.is_empty() || prefix == "." {
            return Ok(self.entries.keys().cloned().collect());
        }
        let key = normalize(prefix)?;
        let dir_prefix = format!("{key}/");
        Ok(self
            .entries
            .keys()
            .filter(|p| **p == key || p.starts_with(&dir_prefix))
            .cloned()
            .collect())
    }

    fn is_executable(&self, path: &str) -> Result<bool, SourceError> {
        Ok(mode_is_executable(self.get(path)?.mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_noise() {
        assert_eq!(
            normalize("./srcs/rust/main.rs").unwrap(),
            "srcs/rust/main.rs"
        );
        assert_eq!(normalize("srcs//rust/").unwrap(), "srcs/rust");
    }

    #[test]
    fn normalize_refuses_traversal() {
        for path in ["../etc/passwd", "srcs/../../etc/passwd", "/etc/passwd"] {
            assert!(
                matches!(normalize(path), Err(SourceError::Escapes { .. })),
                "{path} should be refused"
            );
        }
    }

    #[test]
    fn tar_source_refuses_traversal_on_the_entry_name() {
        let mut source = TarSource::new();
        let err = source.insert("../../etc/passwd", 0o644, b"x".to_vec());
        assert!(matches!(err, Err(SourceError::Escapes { .. })));
        assert!(source.is_empty());
    }

    #[test]
    fn tar_source_reads_lists_and_reports_mode() {
        let source = TarSource::from_entries([
            ("problem.json", 0o644, b"{}".to_vec()),
            ("validation.sh", 0o755, b"#!/bin/sh\n".to_vec()),
            ("srcs/rust/main.rs", 0o644, b"fn main() {}".to_vec()),
        ])
        .unwrap();

        assert_eq!(source.read("problem.json").unwrap(), b"{}");
        assert!(source.is_executable("validation.sh").unwrap());
        assert!(!source.is_executable("problem.json").unwrap());
        assert_eq!(source.list("srcs").unwrap(), vec!["srcs/rust/main.rs"]);
        assert_eq!(source.list("").unwrap().len(), 3);
        assert!(matches!(
            source.read("missing.json"),
            Err(SourceError::NotFound { .. })
        ));
    }
}
