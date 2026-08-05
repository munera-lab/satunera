//! Newtypes over the identifiers that travel between crates.
//!
//! A bare `String` for a problem id survives being passed where a category id
//! was meant. These do not.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }
    };
}

string_id! {
    /// Identifies one problem. Equal to the name of the directory holding its
    /// `problem.json`.
    ProblemId
}

string_id! {
    /// Identifies a catalog category, declared in `config.json`.
    CategoryId
}

string_id! {
    /// Identifies a track, declared in `config.json`.
    TrackId
}

string_id! {
    /// Names a regtest snapshot that the runner restores before the run phase.
    FixtureName
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = ProblemId::new("week-1");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"week-1\"");
        assert_eq!(serde_json::from_str::<ProblemId>("\"week-1\"").unwrap(), id);
    }
}
