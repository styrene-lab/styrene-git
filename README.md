# styrene-git

Repository-signing profile compatibility follows the canonical
[`styrene-identity` policy](https://github.com/styrene-lab/styrene-rs/blob/main/crates/libs/styrene-identity/COMPATIBILITY.md).

Self-certifying, local-first Git collaboration over Styrene-managed network
carriers.

The initial spike implements the network-independent repository state model:

- Deterministic repository identity and identifiers.
- Repository signing keys bound to canonical Styrene identities.
- Threshold-authorized identity transitions.
- Replay-resistant publisher reference transitions.
- Deterministic canonical default-branch selection.
- Shared bare Git object storage with isolated publisher namespaces.
- Transfer-scoped quarantine with object and reachability verification.
- Carrier-neutral manifests and bounded Git pack replication.

Network discovery and transfer remain owned by `styrened`. The repository core
does not depend on RNS, WireGuard, Yggdrasil, I2P, or socket types.

The storage crate invokes the system `git` executable for object validation and
atomic reference transactions. New repositories use SHA-256 object IDs by default.
SHA-1 repositories require the explicit `LegacySha1` compatibility mode and emit a
warning when opened.

Replication transfers contain deterministic CBOR manifests and BLAKE3 integrity
digests. Carrier metadata is not part of repository authorization.

`docs/heartwood-parity.md` records the behavioral checks adopted from Radicle
Heartwood and the Radicle-specific behavior that Styrene intentionally excludes.

## Validation

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Run the layered local gates with `just check` and `just pre-push`. Validate the
machine-readable Heartwood inventory with `just parity-check`. Run generated
identity-history checks with `just property`.

Build the signed functional-harness APK and assemble its SBOM-producing image with
Melange and APKO through Podman:

```bash
bash infra/packaging/build-images.sh git
```

Run `just images` to also package the current sibling `../styrene-rs` worktree and
assemble the daemon lab image. Each build verifies the loaded image metadata,
executables, runtime utilities, and SPDX package inventory. Generated APKs, OCI
archives, signing material, and SBOMs are written under the ignored `artifacts/`
directory. Run `just images-verify` to recheck already-built images and artifacts.
The build script pins multi-architecture Melange and APKO image indexes; use
`MELANGE_IMAGE` or `APKO_IMAGE` only when deliberately testing another tool build.
