//! The typed form of `content/config.json`.
//!
//! One file describes an instance: what the catalog contains and which optional
//! services are deployed. `make config` reads the feature block to write
//! `COMPOSE_PROFILES`, so enabling Lightning happens in exactly one place.

use serde::{Deserialize, Serialize};

use crate::id::{CategoryId, ProblemId, TrackId};

/// Current `config.json` format version.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,

    pub title: String,

    #[serde(default)]
    pub features: Features,

    pub categories: Vec<Category>,

    pub tracks: Vec<Track>,
}

impl Config {
    pub fn category(&self, id: &CategoryId) -> Option<&Category> {
        self.categories.iter().find(|c| &c.id == id)
    }

    pub fn has_category(&self, id: &CategoryId) -> bool {
        self.category(id).is_some()
    }

    /// Every problem id any track references, in declaration order, without
    /// duplicates. The server cross-checks this against what it indexed.
    pub fn referenced_problems(&self) -> Vec<&ProblemId> {
        let mut seen: Vec<&ProblemId> = Vec::new();
        for track in &self.tracks {
            for id in &track.problems {
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
        }
        seen
    }

    /// Compose profiles implied by the feature flags. `core` is always present.
    pub fn compose_profiles(&self) -> Vec<&'static str> {
        let mut profiles = vec!["core"];
        if self.features.lightning {
            profiles.push("lightning");
        }
        if self.features.indexer {
            profiles.push("indexer");
        }
        if self.features.objectstore {
            profiles.push("objectstore");
        }
        if self.features.observability {
            profiles.push("observability");
        }
        profiles
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    #[serde(default)]
    pub lightning: bool,
    #[serde(default)]
    pub indexer: bool,
    #[serde(default)]
    pub objectstore: bool,
    #[serde(default)]
    pub observability: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: CategoryId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An ordered path through the catalog. A problem may appear in several tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub problems: Vec<ProblemId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        serde_json::from_str(
            r#"{
                "schema_version": 1,
                "title": "Bitcoin Online Judge",
                "features": { "lightning": true },
                "categories": [{ "id": "transactions", "title": "Transactions" }],
                "tracks": [
                    { "id": "core", "title": "Core", "problems": ["week-1", "week-2"] },
                    { "id": "extra", "title": "Extra", "problems": ["week-2", "week-3"] }
                ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn profiles_always_include_core() {
        assert_eq!(sample().compose_profiles(), vec!["core", "lightning"]);
    }

    #[test]
    fn referenced_problems_are_deduplicated_in_order() {
        let config = sample();
        let ids: Vec<&str> = config
            .referenced_problems()
            .iter()
            .map(|id| id.as_str())
            .collect();
        assert_eq!(ids, vec!["week-1", "week-2", "week-3"]);
    }

    #[test]
    fn category_lookup_works() {
        assert!(sample().has_category(&CategoryId::new("transactions")));
        assert!(!sample().has_category(&CategoryId::new("lightning")));
    }
}
