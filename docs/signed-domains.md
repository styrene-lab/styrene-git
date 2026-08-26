# Signed And Hashed Domain Inventory

## Scope

This inventory records the cryptographic domains present on 2026-08-25. It covers
`styrene-git-core`, `styrene-git-protocol`, and the Git object checks in
`styrene-git-store`. IPC request correlation and carrier authentication are outside this
inventory because they do not authorize repository state.

Structured Styrene values use numeric-field CBOR arrays. `CanonicalCbor` decodes one value,
rejects trailing bytes, re-encodes the value, and requires byte equality. Callers that
already hold a decoded Rust value can verify its canonical re-encoding. They cannot prove
that omitted source bytes were canonical. Byte-oriented boundaries must call
`from_canonical_bytes` before semantic verification.

Explicit Styrene digests use BLAKE3-256 over `domain || value`. Each current domain
separator ends in `0x00`. Signatures use strict Ed25519 verification. The temporary
`StyreneIdentity` identifier uses a separate construction described below.

## Inventory

| Domain | Version or separator | Signed or hashed value | Context and lineage | Canonical and verification boundary | Evidence and status |
| --- | --- | --- | --- | --- | --- |
| Temporary Styrene Identity identifier | No separator or encoded version | First 16 bytes of SHA-256 over one raw 32-byte Ed25519 public key | No repository, publisher, parent, sequence, epoch, purpose, or algorithm tag | Fixed key length; lowercase 32-character hex parser | `StyreneIdentity::from_public_key`; temporary spike authority tracked by tasks 1.4, 1.7, 1.8, and 1.10 |
| Repository identifier | `styrene/git/repository-id/v1\0`; document version 1; text prefix `styrene:git:v1:` | Canonical initial `IdentityDocument` | Initial delegates, threshold, metadata, and default branch; no parent or sequence field | `IdentityDocument::validate` and canonical encoding before BLAKE3-256 | `IdentityDocument::repository_id`; golden bytes and identifier in `crates/styrene-git-core/tests/spike.rs` |
| Temporary repository signer binding | Payload version 1 and purpose `styrene-repository-signing-v1` | Canonical `SignerBindingPayload` signed by the Identity key | Identity ID, Identity key, repository key, and key epoch; deliberately no repository ID | Strict Ed25519 verification; full imported binding requires canonical decoding | `SignerBinding::issue`, `verify`, and `verify_selected`; temporary profile awaiting authoritative Identity vectors |
| Initial identity-state identifier | `styrene/git/identity-root/v1\0`; document version 1 | Canonical initial `IdentityDocument` | Initial repository policy is implicit in the document; no explicit repository ID, parent, or sequence zero | Document validation and canonical encoding before BLAKE3-256 | `IdentityState::initial`; exercised by identity-history and property tests; no direct golden digest yet |
| Identity-transition approval | Payload version 1; no explicit signature separator | Canonical `IdentityTransitionPayload` signed by each repository key | Repository ID, prior transition, next sequence, and replacement document are signed | Strict Ed25519 verification under a prior-state `SignerSelection` | `IdentityTransition::approve` and `verify`; approval wrapper identity and epoch are not signed and require task 1.11 |
| Identity-transition identifier | `styrene/git/identity-transition/v1\0`; payload version 1 | Canonical complete `IdentityTransition`, including sorted approvals and signatures | Repository ID, parent, sequence, replacement document, and selected proof set | Policy and signature verification precede BLAKE3-256 derivation | `IdentityTransition::verify`; replay, cross-repository, prior-policy, and generated-history tests |
| Reference-transition signature | `styrene/git/ref-transition-signature/v1\0`; payload version 1 | Separator followed by canonical complete `RefTransitionPayload`, signed by the publisher repository key | Repository ID, publisher, key epoch, parent, sequence, and complete sorted ref map | Strict selected-binding verification plus ref and object-ID validation | `RefTransition::signed` and `verify`; golden frame and cross-domain rejection tests are in `refs::tests` |
| Reference-transition identifier | `styrene/git/ref-transition/v1\0`; payload version 1 | Canonical complete `RefTransition`, including signature | All reference signature context and lineage | Canonical encoding before BLAKE3-256; store commit re-verifies transition authority | `RefTransition::transition_id`; replay, lineage, duplicate, and concurrent commit tests |
| Transfer binding digest | `styrene/git/transfer-binding/v1\0` | Canonical embedded `SignerBinding` bytes | Identity and epoch are inside the binding; repository is deliberately absent | Digest check precedes canonical binding decode and selected-binding verification | `Transfer::new`, `validate`, and `apply_transfer`; direct mutation corpus remains in task 5.2b |
| Transfer transition digest | `styrene/git/transfer-transition/v1\0` | Canonical embedded `RefTransition` bytes | Repository, publisher, epoch, parent, and sequence are inside the transition | Digest check precedes canonical transition decode and store verification | `Transfer::new`, `validate`, and `apply_transfer`; malformed envelope and interruption corpus is in `replication.rs` |
| Transfer payload digest | `styrene/git/transfer-payload/v1\0` | Raw Git pack bytes | No repository, publisher, parent, or sequence in this digest | Length and digest checks precede bounded `index-pack`, reachability checks, and transition commit | Modified and oversized payload tests in `crates/styrene-git-protocol/tests/replication.rs` |
| Git object identifier | Git object framing: type, space, decimal length, NUL, then content | Raw Git object framing hashed by repository-selected SHA-256 or legacy SHA-1 | Repository and publisher are supplied only by the containing signed transition | Git `hash-object`, `index-pack`, `rev-list`, and `fsck --strict`; signed IDs carry algorithm and bytes | Store SHA-256, explicit SHA-1, corrupt-object, missing-object, and reachability tests |

## Unsigned Protocol Values

`StateSummary`, `StateWant`, `TransferManifest`, `PayloadDescriptor`, and the outer
`Transfer` are canonical transport values. They are not repository authority. The transfer
manifest and its unkeyed digests are attacker-controlled integrity metadata. Acceptance
comes from canonical embedded values, repository signatures, prior state, object checks,
and atomic store commit.

An attacker can replace a pack with another bounded valid pack that satisfies the same
signed reference targets. The pack can also contain extra valid objects. This does not
authorize a reference, but storage policy must retain size and cleanup bounds.

## Findings And Disposition

| Finding | Effect | Disposition |
| --- | --- | --- |
| Git still defines temporary Identity IDs and binding bytes | A private spike profile could become a second authority | Block production authority on tasks 1.4, 1.7, 1.8, and 1.10 |
| Identity approval identity and epoch are outside signed bytes | A signature can be relabeled if one repository key is reused across accepted identities or epochs | Task 1.11 must bind signer identity and epoch into the approval signing frame |
| Temporary identity approvals lack an explicit signature separator | A type-specific array separates the current schema, but the approval frame is not independently versioned | Task 1.11 adds the authoritative approval frame during Identity migration |
| Identity and reference sequence exhaustion | Maximum sequence previously could panic in checked builds or wrap when overflow checks were disabled | Tasks 1.12 and 2.9 now reject construction and verification at `u64::MAX` |
| Most domains lack direct known-answer and mutation vectors | A field-order or framing change can escape focused review | Tasks 1.8, 2.8, 5.2a, and 5.2b own the missing vectors and mutations |
| Canonical source bytes are visible only at decode boundaries | Calling semantic verification on an already decoded value does not validate original wire bytes | Carrier and IPC adapters must use canonical decode before repository operations; task 5.2a now tests truncation, trailing bytes, and noncanonical outer encoding |
| Identity-transition IDs include the selected approval set | Different valid threshold proof sets for one proposal produce different transition IDs | Retain as the current proof-carrying identity model; reconsider only with an explicit identity-history design change |
| Transfer metadata is not authenticated as a whole | It cannot provide authority or provenance by itself | Accepted by design; `apply_transfer` must continue to derive acceptance from embedded signed state and store checks |

## Final Review Gate

Task 5.3b repeats this inventory after the authoritative Identity migration and all mutation
paths exist. The final review must remove resolved temporary findings, add new domains, and
link each remaining finding to passing evidence or an explicit design decision.
