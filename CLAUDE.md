# Bitcoin Online Judge

A self-hosted, scalable online judge for Bitcoin. Learners solve problems that involve real interaction with a Bitcoin node (building transactions, signing PSBTs, later Lightning channels), and a validator checks the resulting chain state rather than diffing stdout.

Team of 6. Currently pre-MVP: the architecture and task breakdown are done, no code written yet.

## Read these first

| File | What it is |
|---|---|
| `docs/architecture.md` | Full technical architecture: layers, contracts, data model, threat model, scaling path |
| `docs/tasks/README.md` | The 12-task MVP board with ownership and sequencing |
| `docs/tasks/RUST-STACK.md` | Crate choices and four places Rust changes the design |
| `docs/tasks/T*.md` | One file per task: Description, Expected, Verify, Proposed solution |
| `docs/tasks/PHASE2.md` | Explicitly deferred work: do not build these yet |
| `docs/diagrams/*.excalidraw` | Diagram per task, openable at excalidraw.com |

When starting work on a task, read that task's file in full before writing code. The `Verify` section is the acceptance criteria.

## Stack

Rust for everything server-side, TypeScript for the frontend.

Cargo workspace:

```
crates/contracts/   schemas, serde types, ProblemSource trait
crates/validator/   structure rules, no I/O of its own
crates/runner/      bollard sandbox execution
crates/fixtures/    regtest snapshot build and restore
crates/store/       sqlx, migrations, BlobStore
crates/server/      axum, routes, SSE
crates/judgectl/    CLI: validate, verify, fixtures
web/                React + Vite + Monaco
content/            problem repo: week-N/ directories
```

Key crates: `axum`, `tokio`, `bollard`, `sqlx`, `jsonschema`, `clap`, `pulldown-cmark`, `ammonia`, `ulid`, `thiserror`, `tracing`.

## Invariants: do not violate these without a discussion

**`validation.sh` is a frozen contract.** Exit 0 means accepted. The last line of stdout is a single JSON object; everything else is logs. The runner injects a documented set of environment variables. Changing this breaks every authored problem.

**`crates/runner` never depends on `axum` or `sqlx`.** That crate boundary is the phase-2 process split. If those appear in its dependency tree, the boundary has leaked.

**Timeouts are enforced from the runner, never inside the container.** A fork bomb will not honor its own `timeout`. Use `tokio::time::timeout` around the `bollard` wait and `kill_container` on expiry.

**The build phase has no network.** Rust submissions build against a vendored `CARGO_HOME` baked into the toolchain image, with a committed `Cargo.lock` in each scaffold. A missing dependency should produce `CE` naming the allowed crate set.

**Every job gets its own `--internal` Docker network.** Never a shared network, never `network_mode: host`. Assert this in code before user code executes, not only in Compose.

**The runner talks to `docker-socket-proxy`, never the raw socket.** `EXEC=0`, no privileged containers.

**`SE` and `IE` never count against the user.** Infrastructure failures are distinct from wrong answers and trigger automatic retry. Collapsing them into `WA` destroys trust in the judge.

**Problem statements are sanitized with `ammonia` before serving.** Authors are semi-trusted; stored XSS in a statement reaches every learner.

## Open decisions

**`sqlx` backend (T06).** `query!` macros bind to one `DATABASE_URL` at compile time, so dual SQLite/Postgres support costs compile-time SQL checking. Recommendation is SQLite only for the MVP with ANSI-plain SQL, Postgres in phase 2. Not yet ratified, so check for an ADR in `docs/adr/` before assuming.

## Conventions

- `thiserror` in libraries, `anyhow` in binaries.
- Newtypes over ID columns (`SubmissionId`, `UserId`, `ProblemId`), not bare `String`.
- `cargo fmt --check` and `cargo clippy -- -D warnings` are CI gates.
- Commit `.sqlx/` so builds work without a live database; `cargo sqlx prepare --check` is a CI gate.
- Frontend API types are generated from Rust structs with `ts-rs`; CI fails on drift.
- Pin every Docker image by digest, including `bitcoind` and the language toolchains.

## Out of scope for the MVP

Job queue, separate orchestrator process, multi-host runners, MinIO, Lightning, electrs, gVisor, warm node pools, leaderboards. All tracked in `docs/tasks/PHASE2.md`. If a task seems to need one of these, it probably does not. Check the task file for the MVP-shaped alternative.
