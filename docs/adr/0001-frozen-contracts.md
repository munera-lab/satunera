# ADR 0001: The frozen contracts, and what they leave out

- **Status:** accepted
- **Date:** 2026-08-04
- **Task:** T01
- **Supersedes:** nothing

## Context

Three artifacts sit between problem authors and the judge: `config.json`,
`problem.json`, and the `validation.sh` environment and verdict. Five downstream
tasks read them. A field added in week 6 is a migration across four crates and
every authored problem, so the cost of getting these wrong is paid later and by
other people.

The pressure during design was to add fields that "will obviously be needed."
Most of them were not obviously needed; they were obviously *imaginable*.

## Decision

### Verdict transport is the last line of stdout

`validation.sh` prints one JSON object as its last non-blank line. Everything
before it is logs.

Rejected: a file at a known path, which needs a mount, a cleanup path, and a
failure mode when the script never writes it. Rejected: exit code only, which
cannot carry a per-testcase message and so cannot produce the `WA` text that
makes the judge worth using.

The exit code decides acceptance and `ok` must agree with it. The redundancy is
deliberate: it catches the script that detects a failure, prints `ok: false`,
and forgets to `exit 1`. Without the cross-check that submission is accepted.

### The verdict is strict at authoring time, forgiving at run time

`judgectl` holds authors to `verdict.schema.json`, which rejects unknown keys.
The runtime parser ignores unknown keys and reports them as a warning.

A learner must never lose a submission to a typo in someone else's checker, and
an author should hear about the typo before the problem merges. Those two goals
need different strictness, so they get different code paths and one document
explaining the split.

### `ProblemSource` lives in `contracts`, not in `validator`

T02 reads a directory from the CLI; T10 reads an uploaded tarball in memory.
Defining the trait here keeps the validator free of I/O and gives both call
sites one implementation of the rules.

`is_executable` is on the trait because `validation.sh`'s mode bit is a rule the
validator enforces. That single method is why an off-the-shelf VFS crate does
not fit: most drop the mode bits.

### One problem per `week-N/`, marked by `problem.json`

The directory name is the id. Supporting `week-N/<slug>/` later needs a
different walk in the loader and no schema change, so the cheaper layout wins
now.

### Schemas are hand-written, and so are the types

Code generation from JSON Schema to Rust was rejected. The types are small, and
hand-written ones let a field's documentation say what it means to the runner
rather than restating its JSON type. The golden corpus and the round-trip test
are what keep the two in step.

Schema validation runs before serde, and reports every violation at once. Serde
alone produces messages aimed at Rust developers; the audience here is a Bitcoin
educator, and the error message is the product.

## Left out on purpose

| Field | Why not |
|---|---|
| Scoring curves, partial credit, points | Nothing in the MVP scores. `AC`/`WA` is the whole model, and a points field with no consumer becomes a field with an inconsistent consumer |
| Hints, editorial, solution write-ups | Real content will show what shape these want. Guessing now means migrating twice |
| Tags, author names, dates | The catalog filters on category and difficulty. Git already records authorship and dates, and better |
| Per-problem build timeout | The build ceiling tracks toolchain cold-cache time, a property of the image. It is a runner constant (120s) |
| Per-problem `cpus` | The sandbox fixes CPU share. Letting a problem raise its own share is a denial-of-service knob |
| Declared testcase count | The verdict carries `testcase` when it matters. A declared count is a second source of truth that will drift from the script |
| Toolchain image digests | Deployment configuration, pinned in Compose. Content should not decide what image the judge runs |
| Statement metadata (reading time, prerequisites) | Nobody has asked. Cheap to add later, since additions are not breaking |

## Included ahead of need

`services.lnd` and `services.electrs` are in the schema now, false everywhere,
with no code path behind them. The runner branches on them when Lightning lands,
and adding a key to an object is a smaller change than teaching every existing
problem a new required block. This is the one place we accepted forward
declaration, and the golden corpus carries `lightning-preview.json` so the shape
is exercised rather than merely reserved.

`schema_version` is `const 1`. A file claiming version 2 is refused rather than
read optimistically, because a judge that half-understands a manifest produces
verdicts nobody can explain.

## Consequences

- Any change to these three artifacts is a breaking release with a migration for
  the content repo. Additions of optional fields are not.
- The golden corpus in `testdata/golden/` is the shared test surface for T02,
  T07, and T10. A schema change that breaks a downstream consumer fails there
  first.
- `docs/validation-contract.md` and `crates/contracts/src/env.rs` are kept in
  step by a test, not by discipline.
- Cross-file rules the schema cannot express — `problem.json`'s `id` matching
  its directory name, its `category` existing in `config.json`, tracks
  referencing problems that exist — belong to T02 and T07. This ADR does not
  give `contracts` filesystem or catalog awareness to enforce them.
