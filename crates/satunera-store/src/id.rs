//! ULID-backed identifiers for rows this crate owns.
//!
//! `ProblemId` lives in `satunera-contracts` because problems are authored;
//! these three are minted by the judge. ULIDs sort by creation time, so a
//! `SubmissionId` doubles as an idempotency key and an ORDER BY column.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! ulid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Mint a fresh id at the current time.
            pub fn generate() -> Self {
                Self(Ulid::new())
            }

            pub fn as_ulid(&self) -> Ulid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = ulid::DecodeError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(Ulid::from_str(value)?))
            }
        }

        impl From<Ulid> for $name {
            fn from(value: Ulid) -> Self {
                Self(value)
            }
        }
    };
}

ulid_id! {
    /// One row in `users`.
    UserId
}

ulid_id! {
    /// One submitted source. What the learner sees in their history.
    SubmissionId
}

ulid_id! {
    /// One attempt at judging a submission. Retries mint a new one.
    RunId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_creation_time() {
        let earlier = SubmissionId::generate();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let later = SubmissionId::generate();
        assert!(earlier < later);
        assert!(earlier.to_string() < later.to_string());
    }

    #[test]
    fn ids_round_trip_through_strings_and_json() {
        let id = SubmissionId::generate();
        assert_eq!(id.to_string().parse::<SubmissionId>().unwrap(), id);

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<SubmissionId>(&json).unwrap(), id);
    }

    #[test]
    fn garbage_does_not_parse() {
        assert!("not-a-ulid".parse::<SubmissionId>().is_err());
    }
}
