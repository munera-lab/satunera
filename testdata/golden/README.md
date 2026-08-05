# Golden fixtures for the frozen contracts

The shared test surface for T01, and for every downstream consumer of the
schemas: the structure validator (T02), the problem API (T07), and the Author
Studio (T10). A schema change that breaks one of them should fail here first.

```text
valid/                  documents that must load
invalid/                documents that must be rejected
  <name>.json           the sample
  <name>.expect.json    which schema it is written against, the JSON Pointer the
                        violation must name, and why it is wrong
verdict/valid/          verdict objects that satisfy verdict.schema.json
verdict/invalid/        verdict objects that do not
```

Which schema applies to a sample in `valid/` and `invalid/` comes from its file
name: `config-*.json` is a `config.json` document, everything else is a
`problem.json` document. Adding a sample needs no registration step — drop the
file in, add the sidecar if it is an invalid one, and the corpus test picks it
up.

Every invalid sample carries a sidecar because a fixture that fails for an
unexplained reason still passes a test that only checks "this is rejected". The
`pointer` field pins *where* the rejection lands, so a sample that starts
failing for a new reason is a test failure rather than a silent pass.

```bash
cargo test -p satunera-contracts                                                # the whole corpus
jsonschema -i testdata/golden/valid/week-1.json schemas/problem.schema.json
```

The invalid corpus covers required fields, id and name patterns, enum
membership, array emptiness and duplication, unknown fields on every object,
numeric bounds, wrong scalar types, the unsupported-version case, and the
`services.bitcoind` invariant.
