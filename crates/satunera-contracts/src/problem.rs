//! The typed form of `problem.json`.
//!
//! Written by hand rather than generated from the schema. The types are small,
//! and hand-written ones let the field documentation say what a field means to
//! the runner instead of restating its JSON type.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::{CategoryId, FixtureName, ProblemId};

/// Current `problem.json` format version.
pub const PROBLEM_SCHEMA_VERSION: u32 = 1;

/// One problem, as declared by its author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub schema_version: u32,

    /// Must equal the directory name. The validator checks that; the schema
    /// cannot, because it never sees the path.
    pub id: ProblemId,

    pub title: String,

    /// Must be declared in `config.json`. Another cross-file rule the schema
    /// cannot express on its own.
    pub category: CategoryId,

    pub difficulty: Difficulty,

    /// Every language a learner may submit in. Each needs a `srcs/<lang>/`
    /// scaffold and a `solutions/<lang>/` reference solution.
    pub languages: Vec<Language>,

    /// The regtest snapshot restored before `validation.sh` runs.
    pub fixture: FixtureName,

    #[serde(default)]
    pub services: Services,

    #[serde(default)]
    pub limits: Limits,
}

impl Problem {
    /// Directory holding the scaffold handed to a learner.
    pub fn scaffold_dir(&self, language: Language) -> String {
        format!("srcs/{}", language.as_str())
    }

    /// Directory holding the reference solution that `judgectl verify` runs.
    pub fn solution_dir(&self, language: Language) -> String {
        format!("solutions/{}", language.as_str())
    }

    pub fn accepts(&self, language: Language) -> bool {
        self.languages.contains(&language)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];

    pub const fn as_str(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A submission language. The serialized name is also the directory name under
/// `srcs/` and `solutions/`, which is why there is no separate mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Py,
    Cpp,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::Rust, Language::Py, Language::Cpp];

    pub const fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::Py => "py",
            Language::Cpp => "cpp",
        }
    }

    pub fn parse(value: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.as_str() == value)
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which Bitcoin services join the job network.
///
/// `lnd` and `electrs` are here now, and false everywhere, because the runner
/// branches on them when Lightning problems land. They are the one piece of
/// forward declaration in this schema, and the ADR records why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Services {
    /// Always on. A problem with no node is not a problem this judge runs.
    #[serde(default = "yes")]
    pub bitcoind: bool,

    #[serde(default)]
    pub lnd: bool,

    #[serde(default)]
    pub electrs: bool,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            bitcoind: true,
            lnd: false,
            electrs: false,
        }
    }
}

fn yes() -> bool {
    true
}

/// Run-phase ceilings.
///
/// The build-phase timeout is not here. It is a runner constant (120s) because
/// it tracks toolchain cold-cache time, which is a property of the image rather
/// than of the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Wall-clock ceiling for `validation.sh`, enforced by the runner from
    /// outside the container.
    #[serde(default = "default_wall_sec")]
    pub wall_sec: u32,

    /// Memory cap for the user container.
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
}

pub const DEFAULT_WALL_SEC: u32 = 30;
pub const DEFAULT_MEMORY_MB: u32 = 512;

const fn default_wall_sec() -> u32 {
    DEFAULT_WALL_SEC
}

const fn default_memory_mb() -> u32 {
    DEFAULT_MEMORY_MB
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_sec: DEFAULT_WALL_SEC,
            memory_mb: DEFAULT_MEMORY_MB,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_blocks_take_documented_defaults() {
        let problem: Problem = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "id": "week-1",
                "title": "Spend a P2WPKH output",
                "category": "transactions",
                "difficulty": "easy",
                "languages": ["rust"],
                "fixture": "funded-p2wpkh"
            }"#,
        )
        .unwrap();

        assert_eq!(problem.limits, Limits::default());
        assert_eq!(problem.services, Services::default());
        assert_eq!(problem.scaffold_dir(Language::Rust), "srcs/rust");
        assert_eq!(problem.solution_dir(Language::Rust), "solutions/rust");
    }

    #[test]
    fn round_trips_through_json() {
        let problem = Problem {
            schema_version: PROBLEM_SCHEMA_VERSION,
            id: ProblemId::new("week-2"),
            title: "Build and sign a PSBT".into(),
            category: CategoryId::new("psbt"),
            difficulty: Difficulty::Medium,
            languages: vec![Language::Rust, Language::Py, Language::Cpp],
            fixture: FixtureName::new("funded-p2wpkh"),
            services: Services::default(),
            limits: Limits {
                wall_sec: 45,
                memory_mb: 512,
            },
        };

        let json = serde_json::to_string(&problem).unwrap();
        assert_eq!(serde_json::from_str::<Problem>(&json).unwrap(), problem);
    }
}
