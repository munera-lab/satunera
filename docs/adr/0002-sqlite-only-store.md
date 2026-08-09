# ADR 0002: SQLite only, checked queries, Postgres deferred

- **Status:** accepted
- **Date:** 2026-08-09
- **Task:** T06
- **Supersedes:** nothing

## Context

`sqlx`'s `query!` macros check every statement against a real schema at compile
time, which is most of the reason `sqlx` was chosen over an ORM. That checking
binds to a single backend: one `DATABASE_URL`, one dialect. Supporting SQLite
and Postgres from the same macros does not work, and `sqlx::Any` throws away
the typed row mapping that made the macros worth having.

So the choice is between compile-time checking with one backend now, or two
backends now with every query checked only at run time — in a crate that T07
and T08 both compile against, written before either consumer exists.

The write volume is a classroom: tens of submissions a minute at the worst
imaginable peak. Nothing about the MVP's load needs Postgres.

## Decision

### SQLite is the only backend in the MVP

WAL mode, `busy_timeout` set through `SqliteConnectOptions`, foreign keys on.
One file, no container, which keeps self-hosting at genuinely one command.

Postgres arrives in phase 2 behind the `Store` trait, as a second
implementation passing the same test suite. It is not built now, and no code
pretends it is.

### The SQL stays ANSI-plain so the port stays an afternoon

The deferred migration is cheap only if the SQL does not grow SQLite habits.
Concretely:

- no `INSERT OR REPLACE`, no `ON CONFLICT` shorthand beyond what Postgres
  shares, no SQLite date functions;
- explicit types on every column, even where SQLite would not care;
- IDs and timestamps stored as `TEXT` (ULIDs and RFC 3339), never as SQLite's
  flexible-typed integers;
- JSON stored in `TEXT` columns and parsed in Rust, not queried with `json_*`
  functions.

### Every read and write goes through the `Store` trait

Consumers (T07, T08) hold a `dyn Store` (or a generic), never a `SqlitePool`.
The trait is the seam the Postgres implementation drops into, and it is also
what lets tests run against an in-memory database without a fixture file.

`BlobStore` is the same shape for bytes: `put`/`get`/`uri`, filesystem
implementation now, S3 in phase 2. Source blobs and logs never go in the
database, so the database stays small enough that "copy one file" is a
complete backup strategy.

### `.sqlx/` is committed and checked in CI

`cargo sqlx prepare` writes offline query metadata so every build works
without a live database. A stale `.sqlx/` is the most common way this crate
breaks a teammate's build, so `cargo sqlx prepare --check` is a CI gate, not
advice.

## Rejected

**Both backends from day one** (the original Go plan's advice). With `sqlx`
the advice inverts: shipping both means giving up compile-time checking on
every query, which costs more every day than the one-time port risk it
insures against.

**`sqlx::Any`.** Runtime-chosen backend, runtime-checked queries, untyped
rows. Worst of both.

**An ORM (SeaORM, Diesel).** The query surface is a dozen statements. An ORM
buys migration DSLs and query builders the crate does not need, at the price
of a second language between the code and the schema.

## Consequences

- The Postgres port is a phase-2 task with a known shape: second
  implementation of `Store`, same suite, plus a migration script for the data.
- Anything that would fork the SQL dialect (upserts, date arithmetic, JSON
  queries) must be solved in Rust instead, and reviewers should treat dialect
  creep in migrations as a bug.
- The database cannot be shared by two writer processes on different hosts.
  That is fine: the phase-2 process split keeps a single writer, and the
  multi-host story arrives with Postgres.
