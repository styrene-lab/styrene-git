# Heartwood Behavioral Parity

## Reference

Styrene Git uses Radicle Heartwood as a behavioral reference. Heartwood is not the
protocol authority and is not a patch source for this review.

- Repository: `https://github.com/radicle-dev/heartwood`
- Reviewed revision: `3da781f9e17789a030703db8f61ca2abe21b78a7`
- Revision date: 2026-08-23
- Review date: 2026-08-25
- License: `MIT OR Apache-2.0`

The machine-readable inventory is `parity/heartwood.toml`. Run
`just parity-check` after changing an upstream decision, local evidence path, or gate.

## Provenance And Licensing

A current-tree audit on 2026-08-25 found no Heartwood source files, code snippets, tests,
fixtures, or dependencies imported into Styrene Git. The references in this document record
behavioral and test-dimension review at the revision above. They do not claim source-code
incorporation. This finding is limited to the available tree because repository history was
not available for review.

The audit identified no Heartwood material redistributed by the current tree and no current
Heartwood redistribution-notice obligation. If a later change copies or adapts Heartwood
material, Styrene Git selects Heartwood's MIT option. Before that change merges, it must
record the upstream file and revision. It must also preserve the complete Heartwood MIT
notice, including `Copyright (c) 2021 The Radicle Foundation`.

## Authority Order

1. Styrene OpenSpec requirements define repository behavior.
2. Styrene Identity defines user and signing authority.
3. Git defines object and remote-helper behavior.
4. Heartwood supplies test dimensions and implementation evidence.

Heartwood behavior does not override a Styrene requirement or trust boundary.

## Decisions

### Adopt

- Model-generated identity histories and invariant checks.
- Generated signed-reference maps and transition chains.
- Deterministic real-Git fixtures with fixed keys, timestamps, locale, and author data.
- Remote-helper parser tests for capabilities, list, fetch, push, force, deletion, options, and malformed input.
- Black-box Git tests for clone, fetch, push, amend, divergence, force-with-lease, deletion, tags, and checkout.
- Duplicate, concurrent, stale, and bounded replication checks.
- Cross-platform Git subprocess tests on Linux, macOS, and Windows.
- Explicit slow, adversarial, and crash-recovery gates.

### Defer

- Canonical wildcard and special-reference rules. Styrene currently derives only the
  configured default branch.
- Expensive daemon lifecycle tests until `styrened` owns a durable Git operation model.

### Skip

- Radicle COB identity voting, redaction, and sibling-resolution mechanics.
- Radicle node IDs as repository authority.
- `rad/sigrefs` commits, migration levels, and downgrade behavior.
- Radicle gossip, seeding, routing, and peer-block policy in repository crates.
- Heartwood pack installation into the production object database before signed-state
  validation.

## Gate Layers

| Gate | State | Purpose |
| --- | --- | --- |
| `just check` | Active | Format, compile, rustdoc, parity inventory, and OpenSpec structure. |
| `just pre-push` | Active | Fast gate plus all tests and warning-denied Clippy. |
| `just container` | Active | Clean Linux build, tests, and Clippy. |
| GitHub Actions matrix | Active | Linux, macOS, and Windows workspace tests. |
| `just property` | Active | Generated identity histories. Signed-reference chains remain open. |
| `git-black-box` | Planned | Real `styrene://` helper and daemon Git workflows. |
| `quarantine-adversarial` | Planned | Corrupt packs, bad deltas, missing objects, interruption, and cleanup. |
| `crash-recovery` | Planned | Process failure across promotion and ref-transaction boundaries. |
| `daemon-lifecycle` | Planned | Durable queue, cancellation, retry, restart, and observation behavior. |

## Priority Order

1. Extend property tests from identity histories to signed-reference chains.
2. Build the remote-helper parser and deterministic black-box Git fixture.
3. Add malformed pack and quarantine cleanup corpora.
4. Add crash injection around object promotion and ref transactions.
5. Add durable daemon operation tests after the IPC and service boundaries exist.

## Upstream Caveats

Heartwood has broad real-Git and node coverage, but it does not prove Styrene quarantine
semantics. Its fetch path can write pack objects into the production object database before
signed-state validation. Styrene must retain transfer-scoped quarantine and atomic accepted
reference updates.

Heartwood also has no focused process-crash matrix for object promotion and reference
transactions. Styrene needs independent recovery tests for those boundaries.
