//! Names of the environment variables the runner injects into the run phase.
//!
//! The runner sets them and `validation.sh` reads them, and those two live in
//! different crates written by different people. Keeping the names here as
//! constants means a rename is a compile error on one side rather than a silent
//! empty string on the other.
//!
//! `docs/validation-contract.md` documents what each one carries. A test in
//! this crate fails if a constant here is missing from that document.

/// Prefixes reserved by the judge. A problem author must not expect any other
/// variable to survive into the container, and must not set these themselves.
pub const RESERVED_PREFIXES: [&str; 2] = ["JUDGE_", "BITCOIN_"];

/// The problem being judged, e.g. `week-1`.
pub const JUDGE_PROBLEM_ID: &str = "JUDGE_PROBLEM_ID";
/// ULID of the submission, unique per attempt and stable across retries.
pub const JUDGE_SUBMISSION_ID: &str = "JUDGE_SUBMISSION_ID";
/// Language the learner submitted in: `rust`, `py`, or `cpp`.
pub const JUDGE_LANGUAGE: &str = "JUDGE_LANGUAGE";
/// Absolute path of the materialized problem directory, and the working
/// directory `validation.sh` starts in.
pub const JUDGE_WORKDIR: &str = "JUDGE_WORKDIR";
/// Absolute path of the learner's sources for this language.
pub const JUDGE_SRC_DIR: &str = "JUDGE_SRC_DIR";
/// Command that runs the already-built submission. `validation.sh` invokes this
/// rather than knowing how each language is launched.
pub const JUDGE_RUN: &str = "JUDGE_RUN";
/// Wall-clock ceiling in seconds, so a script can budget its own waits. The
/// runner enforces the ceiling regardless of what the script does with it.
pub const JUDGE_WALL_SEC: &str = "JUDGE_WALL_SEC";
/// Name of the regtest snapshot restored before this run.
pub const JUDGE_FIXTURE: &str = "JUDGE_FIXTURE";
/// The only writable path in the container: a 64 MB `noexec` tmpfs.
pub const JUDGE_TMPDIR: &str = "JUDGE_TMPDIR";

/// Always `regtest` in the MVP.
pub const BITCOIN_NETWORK: &str = "BITCOIN_NETWORK";
/// Hostname of this job's node on the job-private network.
pub const BITCOIN_RPC_HOST: &str = "BITCOIN_RPC_HOST";
pub const BITCOIN_RPC_PORT: &str = "BITCOIN_RPC_PORT";
pub const BITCOIN_RPC_USER: &str = "BITCOIN_RPC_USER";
pub const BITCOIN_RPC_PASSWORD: &str = "BITCOIN_RPC_PASSWORD";
/// `http://$BITCOIN_RPC_HOST:$BITCOIN_RPC_PORT`, for clients that want one string.
pub const BITCOIN_RPC_URL: &str = "BITCOIN_RPC_URL";
/// Name of the wallet the fixture funded.
pub const BITCOIN_WALLET: &str = "BITCOIN_WALLET";
/// A ready-made `bitcoin-cli` invocation with connection flags already applied.
pub const BITCOIN_CLI: &str = "BITCOIN_CLI";

/// Set only when `services.lnd` is true.
pub const LND_RPC_HOST: &str = "LND_RPC_HOST";
pub const LND_GRPC_PORT: &str = "LND_GRPC_PORT";
pub const LND_TLS_CERT_PATH: &str = "LND_TLS_CERT_PATH";
pub const LND_MACAROON_PATH: &str = "LND_MACAROON_PATH";

/// Set only when `services.electrs` is true.
pub const ELECTRS_HOST: &str = "ELECTRS_HOST";
pub const ELECTRS_PORT: &str = "ELECTRS_PORT";

/// Injected into every run.
pub const ALWAYS_INJECTED: [&str; 17] = [
    JUDGE_PROBLEM_ID,
    JUDGE_SUBMISSION_ID,
    JUDGE_LANGUAGE,
    JUDGE_WORKDIR,
    JUDGE_SRC_DIR,
    JUDGE_RUN,
    JUDGE_WALL_SEC,
    JUDGE_FIXTURE,
    JUDGE_TMPDIR,
    BITCOIN_NETWORK,
    BITCOIN_RPC_HOST,
    BITCOIN_RPC_PORT,
    BITCOIN_RPC_USER,
    BITCOIN_RPC_PASSWORD,
    BITCOIN_RPC_URL,
    BITCOIN_WALLET,
    BITCOIN_CLI,
];

/// Injected only when `services.lnd` is enabled.
pub const LND_INJECTED: [&str; 4] = [
    LND_RPC_HOST,
    LND_GRPC_PORT,
    LND_TLS_CERT_PATH,
    LND_MACAROON_PATH,
];

/// Injected only when `services.electrs` is enabled.
pub const ELECTRS_INJECTED: [&str; 2] = [ELECTRS_HOST, ELECTRS_PORT];

/// Every variable the judge may inject, in documentation order.
pub fn all() -> Vec<&'static str> {
    ALWAYS_INJECTED
        .iter()
        .chain(LND_INJECTED.iter())
        .chain(ELECTRS_INJECTED.iter())
        .copied()
        .collect()
}
