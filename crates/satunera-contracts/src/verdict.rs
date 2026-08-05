//! The result side of the frozen contract.
//!
//! `validation.sh` communicates two things: an exit code, which decides
//! acceptance, and a single JSON object on the last line of stdout, which
//! explains it. Everything else on stdout is logs.
//!
//! Parsing here is deliberately more forgiving than the schema. `judgectl`
//! holds authors to `schemas/verdict.schema.json` while they are writing;
//! at run time a stray extra key must not cost a learner their submission.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The judge's answer for one submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    /// Accepted.
    Ac,
    /// Wrong answer: `validation.sh` ran and rejected the submission.
    Wa,
    /// Exceeded the wall-clock ceiling.
    Tle,
    /// Hit the memory cap.
    Mle,
    /// The submission crashed or exited non-zero outside `validation.sh`.
    Re,
    /// The submission did not build.
    Ce,
    /// System error: the judge itself failed.
    Se,
    /// Internal error: the problem's own tooling failed, e.g. `validation.sh`
    /// printed something that is not a verdict.
    Ie,
}

impl Verdict {
    pub const ALL: [Verdict; 8] = [
        Verdict::Ac,
        Verdict::Wa,
        Verdict::Tle,
        Verdict::Mle,
        Verdict::Re,
        Verdict::Ce,
        Verdict::Se,
        Verdict::Ie,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Verdict::Ac => "AC",
            Verdict::Wa => "WA",
            Verdict::Tle => "TLE",
            Verdict::Mle => "MLE",
            Verdict::Re => "RE",
            Verdict::Ce => "CE",
            Verdict::Se => "SE",
            Verdict::Ie => "IE",
        }
    }

    pub fn parse(value: &str) -> Option<Verdict> {
        Verdict::ALL.into_iter().find(|v| v.as_str() == value)
    }

    pub const fn is_accepted(self) -> bool {
        matches!(self, Verdict::Ac)
    }

    /// Whether the failure was the judge's or the author's rather than the
    /// learner's. `SE` and `IE` never count against a user, and collapsing them
    /// into `WA` is how a judge loses the room's trust.
    pub const fn is_infrastructure(self) -> bool {
        matches!(self, Verdict::Se | Verdict::Ie)
    }

    /// True when the verdict reflects something the learner's code did.
    pub const fn counts_against_user(self) -> bool {
        !self.is_infrastructure()
    }

    /// Infrastructure failures are retried automatically, up to
    /// [`MAX_AUTOMATIC_RETRIES`] times.
    pub const fn is_retryable(self) -> bool {
        self.is_infrastructure()
    }
}

/// How many times the runner re-runs a job that ended in `SE` or `IE`.
pub const MAX_AUTOMATIC_RETRIES: u32 = 2;

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Longest `message` the judge forwards. Longer messages are truncated at run
/// time; `judgectl` rejects them at authoring time so the author finds out
/// first.
pub const MAX_MESSAGE_CHARS: usize = 500;

/// The JSON object on the last line of stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationOutput {
    /// Whether the submission satisfied the problem. Must agree with the exit
    /// code; see [`ValidationOutput::reconcile`].
    pub ok: bool,

    /// Why it failed, in the learner's terms. Required when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// 1-based index of the failing check, for problems that run several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testcase: Option<u32>,

    /// Keys the judge did not recognize. Not an error: they are surfaced as a
    /// warning so a typo like `msg` is visible without failing the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_keys: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerdictError {
    #[error(
        "validation.sh produced no output. \
         The problem's validation.sh must print a JSON verdict as its last line of stdout."
    )]
    NoOutput,

    #[error(
        "validation.sh: last line of stdout is not JSON: {excerpt}\n\
         This is a bug in the problem's validation.sh, not in the submission."
    )]
    NotJson { excerpt: String },

    #[error(
        "validation.sh: last line of stdout is JSON but not an object: {excerpt}\n\
         This is a bug in the problem's validation.sh, not in the submission."
    )]
    NotAnObject { excerpt: String },

    #[error(
        "validation.sh: verdict is missing the required `ok` field.\n\
         This is a bug in the problem's validation.sh, not in the submission."
    )]
    MissingOk,

    #[error(
        "validation.sh: verdict field `{field}` must be {expected}.\n\
         This is a bug in the problem's validation.sh, not in the submission."
    )]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },

    #[error(
        "validation.sh: verdict says the submission failed but carries no `message`.\n\
         Every rejection must tell the learner what is wrong."
    )]
    MissingMessage,

    #[error(
        "validation.sh: verdict says ok={ok} but the script exited with {exit_code}. \
         The exit code decides acceptance (0 accepts, non-zero rejects) and the two must agree.\n\
         This is a bug in the problem's validation.sh, not in the submission."
    )]
    ExitCodeMismatch { ok: bool, exit_code: i32 },
}

impl ValidationOutput {
    /// An acceptance with no further detail.
    pub fn accepted() -> Self {
        Self {
            ok: true,
            message: None,
            testcase: None,
            unknown_keys: Vec::new(),
        }
    }

    /// A rejection carrying the author's message.
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            testcase: None,
            unknown_keys: Vec::new(),
        }
    }

    /// Pull the verdict out of the captured stdout: the last non-blank line,
    /// with everything before it treated as logs.
    pub fn from_stdout(stdout: &str) -> Result<Self, VerdictError> {
        let line = stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .ok_or(VerdictError::NoOutput)?;
        Self::from_line(line)
    }

    /// Parse one line as a verdict object.
    pub fn from_line(line: &str) -> Result<Self, VerdictError> {
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).map_err(|_| VerdictError::NotJson {
                excerpt: excerpt(line),
            })?;

        let object = value.as_object().ok_or_else(|| VerdictError::NotAnObject {
            excerpt: excerpt(line),
        })?;

        let ok = match object.get("ok") {
            None => return Err(VerdictError::MissingOk),
            Some(v) => v.as_bool().ok_or(VerdictError::WrongType {
                field: "ok",
                expected: "true or false",
            })?,
        };

        let message = match object.get("message") {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => Some(truncate(v.as_str().ok_or(VerdictError::WrongType {
                field: "message",
                expected: "a string",
            })?)),
        };

        let testcase = match object.get("testcase") {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => {
                let n = v.as_u64().ok_or(VerdictError::WrongType {
                    field: "testcase",
                    expected: "a positive integer",
                })?;
                if n == 0 || n > u32::MAX as u64 {
                    return Err(VerdictError::WrongType {
                        field: "testcase",
                        expected: "a positive integer",
                    });
                }
                Some(n as u32)
            }
        };

        if !ok && message.as_ref().is_none_or(|m| m.trim().is_empty()) {
            return Err(VerdictError::MissingMessage);
        }

        let unknown_keys = object
            .keys()
            .filter(|k| !matches!(k.as_str(), "ok" | "message" | "testcase"))
            .cloned()
            .collect();

        Ok(Self {
            ok,
            message,
            testcase,
            unknown_keys,
        })
    }

    /// Confirm the exit code and the `ok` field tell the same story.
    ///
    /// This catches the common authoring bug where a script detects a failure,
    /// prints `ok: false`, and forgets to `exit 1`. Without the cross-check
    /// every such submission would be accepted.
    pub fn reconcile(&self, exit_code: i32) -> Result<(), VerdictError> {
        if self.ok == (exit_code == 0) {
            Ok(())
        } else {
            Err(VerdictError::ExitCodeMismatch {
                ok: self.ok,
                exit_code,
            })
        }
    }

    /// The verdict this output implies, once the exit code agrees with it.
    pub fn verdict(&self) -> Verdict {
        if self.ok {
            Verdict::Ac
        } else {
            Verdict::Wa
        }
    }
}

fn truncate(message: &str) -> String {
    match message.char_indices().nth(MAX_MESSAGE_CHARS) {
        None => message.to_string(),
        Some((index, _)) => format!("{}…", &message[..index]),
    }
}

fn excerpt(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(120) {
        None => line.to_string(),
        Some((index, _)) => format!("{}…", &line[..index]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_line_wins_and_earlier_lines_are_logs() {
        let stdout = "checking outputs\n{\"ok\": true}\n\n";
        let output = ValidationOutput::from_stdout(stdout).unwrap();
        assert!(output.ok);
        assert_eq!(output.verdict(), Verdict::Ac);
    }

    #[test]
    fn a_log_line_that_looks_like_json_does_not_win() {
        let stdout = "{\"ok\": true}\n{\"ok\": false, \"message\": \"wrong outpoint\"}";
        let output = ValidationOutput::from_stdout(stdout).unwrap();
        assert!(!output.ok);
        assert_eq!(output.message.as_deref(), Some("wrong outpoint"));
    }

    #[test]
    fn rejection_without_a_message_is_an_authoring_bug() {
        assert_eq!(
            ValidationOutput::from_line("{\"ok\": false}"),
            Err(VerdictError::MissingMessage)
        );
    }

    #[test]
    fn malformed_json_names_the_authors_script() {
        let err = ValidationOutput::from_stdout("not json at all").unwrap_err();
        assert!(err.to_string().contains("validation.sh"));
    }

    #[test]
    fn empty_stdout_is_an_error_not_an_acceptance() {
        assert_eq!(
            ValidationOutput::from_stdout("  \n\n"),
            Err(VerdictError::NoOutput)
        );
    }

    #[test]
    fn unknown_keys_are_warned_about_not_fatal() {
        let output =
            ValidationOutput::from_line("{\"ok\": true, \"msg\": \"typo\", \"detail\": 1}")
                .unwrap();
        assert!(output.ok);
        assert_eq!(output.unknown_keys, vec!["detail", "msg"]);
    }

    #[test]
    fn exit_code_and_ok_must_agree() {
        assert!(ValidationOutput::accepted().reconcile(0).is_ok());
        assert!(ValidationOutput::rejected("nope").reconcile(1).is_ok());
        assert_eq!(
            ValidationOutput::rejected("nope").reconcile(0),
            Err(VerdictError::ExitCodeMismatch {
                ok: false,
                exit_code: 0
            })
        );
        assert!(ValidationOutput::accepted().reconcile(1).is_err());
    }

    #[test]
    fn long_messages_are_truncated_rather_than_rejected() {
        let long = "x".repeat(MAX_MESSAGE_CHARS + 50);
        let line = serde_json::json!({ "ok": false, "message": long }).to_string();
        let output = ValidationOutput::from_line(&line).unwrap();
        let message = output.message.unwrap();
        assert_eq!(message.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn infrastructure_verdicts_never_count_against_the_user() {
        for verdict in Verdict::ALL {
            let infra = matches!(verdict, Verdict::Se | Verdict::Ie);
            assert_eq!(verdict.is_infrastructure(), infra);
            assert_eq!(verdict.counts_against_user(), !infra);
            assert_eq!(verdict.is_retryable(), infra);
        }
    }

    #[test]
    fn verdict_codes_round_trip() {
        for verdict in Verdict::ALL {
            let json = serde_json::to_string(&verdict).unwrap();
            assert_eq!(json, format!("\"{}\"", verdict.as_str()));
            assert_eq!(Verdict::parse(verdict.as_str()), Some(verdict));
        }
    }
}
