# The `validation.sh` contract

This document is frozen. Every authored problem depends on it, and changing it
after week 3 means editing every `validation.sh` in the content repo. Treat an
addition as cheap and a change or removal as a breaking release.

`validation.sh` is the author-supplied checker. The runner executes it inside
the sandbox after the learner's code has been built, with a live regtest node
reachable on a job-private network. It decides whether the submission solved the
problem, and it says why when the answer is no.

Two rules carry the whole contract:

1. **The exit code decides.** `0` accepts, anything else rejects.
2. **The last non-blank line of stdout is a single JSON object.** Everything
   before it is logs and is streamed to the learner as-is.

## The problem directory

```text
week-1/
  problem.json           manifest, validated against schemas/problem.schema.json
  README.md              statement, rendered to HTML and sanitized at boot
  validation.sh          this contract; must be executable (mode bit 0o111)
  srcs/<lang>/           scaffold handed to the learner, one per declared language
  solutions/<lang>/      reference solution, must earn AC, one per declared language
```

`<lang>` is exactly the string in `problem.json`'s `languages` array: `rust`,
`py`, or `cpp`. Anything else in the directory is the author's business, subject
to the safety rules the validator enforces (no symlinks leaving the directory,
no file over 10 MB, no nested `.git`).

## Execution

- Working directory: `$JUDGE_WORKDIR`, the materialized copy of the problem
  directory. The original content repo is never mounted.
- Interpreter: the script's own shebang. Use `#!/usr/bin/env bash`.
- The build phase has already run. The learner's code is compiled; launch it
  with `$JUDGE_RUN` rather than assuming a path or an interpreter.
- Wall-clock ceiling: `limits.wall_sec` from `problem.json`, enforced by the
  runner from outside the container. A `timeout` call inside the script is
  redundant and does not protect anything, because a runaway process does not
  cooperate with it.
- Filesystem: read-only except `$JUDGE_TMPDIR`, a 64 MB `noexec` tmpfs.
- Network: a private `--internal` network holding this job's Bitcoin services
  and nothing else. There is no route to the internet and no route to another
  job.
- User: unprivileged (`65534`), all capabilities dropped.

## Injected environment

Always set:

| Variable | Example | Meaning |
|---|---|---|
| `JUDGE_PROBLEM_ID` | `week-1` | The problem being judged |
| `JUDGE_SUBMISSION_ID` | `01J9F...` | ULID of this submission, stable across retries |
| `JUDGE_LANGUAGE` | `rust` | Language the learner submitted in |
| `JUDGE_WORKDIR` | `/judge` | Materialized problem directory, and the cwd |
| `JUDGE_SRC_DIR` | `/judge/srcs/rust` | The learner's sources for this language |
| `JUDGE_RUN` | `/judge/build/solution` | Command that runs the built submission |
| `JUDGE_WALL_SEC` | `30` | Wall-clock ceiling, so the script can budget its own waits |
| `JUDGE_FIXTURE` | `funded-p2wpkh` | Regtest snapshot restored before this run |
| `JUDGE_TMPDIR` | `/tmp` | The only writable path |
| `BITCOIN_NETWORK` | `regtest` | Always `regtest` in the MVP |
| `BITCOIN_RPC_HOST` | `bitcoind` | This job's node on the job network |
| `BITCOIN_RPC_PORT` | `18443` | |
| `BITCOIN_RPC_USER` | `judge` | |
| `BITCOIN_RPC_PASSWORD` | `judge` | Per-job credential, not a shared secret |
| `BITCOIN_RPC_URL` | `http://bitcoind:18443` | For clients that want one string |
| `BITCOIN_WALLET` | `learner` | Wallet the fixture funded |
| `BITCOIN_CLI` | `bitcoin-cli -regtest -rpcconnect=…` | Ready-made invocation with connection flags applied |

Set only when `problem.json` enables the service:

| Variable | Condition |
|---|---|
| `LND_RPC_HOST` | `services.lnd` |
| `LND_GRPC_PORT` | `services.lnd` |
| `LND_TLS_CERT_PATH` | `services.lnd` |
| `LND_MACAROON_PATH` | `services.lnd` |
| `ELECTRS_HOST` | `services.electrs` |
| `ELECTRS_PORT` | `services.electrs` |

`HOME` is `$JUDGE_TMPDIR` and `PATH` is the toolchain image's. Nothing else
carries over: no host environment, no server configuration, no other
submission's state.

The `JUDGE_` and `BITCOIN_` prefixes are reserved. Do not set your own variables
with those prefixes, and do not rely on any variable not listed above.

The learner's process inherits this environment, so the `BITCOIN_*` variables
are also how a submission reaches its node. Say so in the statement.

## The verdict object

The last non-blank line of stdout, one JSON object, validated against
[`schemas/verdict.schema.json`](../schemas/verdict.schema.json).

```json
{"ok": false, "message": "Transaction spends the wrong outpoint", "testcase": 2}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `ok` | boolean | yes | Whether the submission solved the problem |
| `message` | string | when `ok` is false | Why it failed, in the learner's terms. Max 500 characters |
| `testcase` | integer ≥ 1 | no | Which check failed, for problems that run several |

`ok` must agree with the exit code: `true` with `0`, `false` with non-zero. A
disagreement is an authoring bug — usually a script that detected a failure and
forgot to `exit 1` — and yields `IE` rather than silently accepting the
submission.

Unknown keys are refused by `judgectl` while you author, and ignored (with a
warning) at run time. A typo like `msg` will not cost a learner a submission,
but you will find out about it before the problem merges.

Write `message` to name what is wrong without giving away the fix.
`Transaction spends the wrong outpoint` teaches. `use listunspent[0] instead of
[1]` does not.

## Verdicts

The script produces only `ok`. The runner turns it into a verdict:

| Verdict | Meaning | Counts against the learner |
|---|---|---|
| `AC` | Exit 0, `ok: true` | — |
| `WA` | Exit non-zero, `ok: false` | yes |
| `TLE` | Wall-clock ceiling exceeded | yes |
| `MLE` | Memory cap hit | yes |
| `RE` | The submission crashed | yes |
| `CE` | The submission did not build | yes |
| `SE` | The judge itself failed | **no**, retried automatically |
| `IE` | The problem's tooling failed: no verdict line, malformed JSON, exit code disagreeing with `ok` | **no**, retried automatically |

`SE` and `IE` never count against a learner and never appear as `WA`. An
infrastructure failure that looks like a wrong answer destroys trust in the
judge faster than any bug.

`IE` is the author's signal. If a problem produces `IE` in CI, the checker is
broken, not the submission.

## A worked example

```bash
#!/usr/bin/env bash
# week-1: spend the funded P2WPKH output to the target address.
set -uo pipefail

fail() {
  # One JSON object, last line, then a non-zero exit. Both are required.
  jq -cn --arg m "$1" '{ok: false, message: $m}'
  exit 1
}

# Record the state to assert against before anything runs.
funded_txid=$($BITCOIN_CLI -rpcwallet="$BITCOIN_WALLET" listunspent | jq -r '.[0].txid')
echo "funded outpoint: $funded_txid"          # a log line, ignored by the judge

# Run the learner's code. It reaches the node through the same BITCOIN_* vars.
if ! "$JUDGE_RUN" > "$JUDGE_TMPDIR/run.log" 2>&1; then
  cat "$JUDGE_TMPDIR/run.log"
  fail "Your program exited with an error before broadcasting anything"
fi
cat "$JUDGE_TMPDIR/run.log"

# Assert on chain state, never on what the program printed.
txids=$($BITCOIN_CLI getrawmempool | jq -r '.[]')
[ -n "$txids" ] || fail "No transaction reached the mempool"

txid=$(printf '%s\n' "$txids" | head -n1)
spent=$($BITCOIN_CLI getrawtransaction "$txid" true | jq -r '.vin[0].txid')
[ "$spent" = "$funded_txid" ] || fail "Transaction spends the wrong outpoint"

jq -cn '{ok: true}'
exit 0
```

Three things that example is demonstrating:

- **Check chain state, not stdout.** `getrawtransaction` tells you what actually
  happened. Parsing a printed txid tests string formatting, not Bitcoin.
- **Accept more than one route to the answer.** A learner who builds the
  transaction by hand and one who uses a wallet RPC both pass if the broadcast
  transaction is correct.
- **Every failure path emits a message and a non-zero exit.** A `fail` helper is
  the cheapest way to make that true on every branch.

## Checking your work

```bash
judgectl validate content/week-1   # structure, schema, shellcheck, safety rules
judgectl verify   content/week-1   # runs every reference solution, requires AC
```

CI runs both on every content PR. `verify` is the one that catches an
unsolvable problem, and it is the reason reference solutions are a deliverable
rather than a nicety.
