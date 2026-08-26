# Bootstrap Self-Certifying Repositories Design

## Goals

1. Repository truth is independent of storage host and network carrier.
2. Styrene Identity is the only user and device identity authority.
3. Git remains the object store and working-copy interface.
4. `styrened` remains the sole owner of discovery, network sessions, tunnels, and policy.
5. Every accepted mutable state is an ordered, replay-resistant signed transition.
6. Local pushes work without network connectivity.

## Component boundary

```text
git
 |
 | Git remote-helper protocol
 v
git-remote-styrene
 |
 | typed authenticated local IPC
 v
styrened GitService
 |                         styrened synchronization coordinator
 | repository port          | peer/carrier/deadline policy
 v                          v
styrene-git core/store <--- carrier-neutral transfer ----> RNS/WG/Yggdrasil/I2P
```

The `styrene-git` domain must not depend on `styrene-rns`, `styrene-tunnel`, socket
addresses, or daemon service types. A `styrened` adapter implements repository ports and
maps synchronization operations to existing transport, Resource, tunnel, content, RBAC,
and observation facilities.

`git-remote-styrene` is intentionally thin. It translates Git remote-helper requests to
local IPC and translates responses back to Git. It never resolves or contacts network
peers.

## Initial workspace shape

```text
styrene-git-core       identity, policy, transitions, canonicalization
styrene-git-store      bare Git storage, namespaces, quarantine, transactions
styrene-git-protocol   carrier-neutral offers, wants, manifests, and outcomes
styrene-git-ipc        daemon-facing request and observation contract
git-remote-styrene     Git remote-helper binary
styrene-git            operator/developer CLI
```

The existing `styrene-forge` crate in `styrene-rs` remains a contract for centralized
Forgejo, GitHub, and GitLab APIs. It is not reused as the repository engine.

## Repository identity format

Version 1 uses RFC 8949 deterministic CBOR for signed domain values. Human-facing JSON or
TOML is a projection and is never hashed or signed directly. Golden fixtures pin canonical
bytes.

The initial identity document contains:

- Schema and identifier-algorithm versions
- Project name and description
- Git default branch
- Visibility declaration
- Sorted unique canonical Styrene delegate identities
- Delegate threshold
- Extensible payload map with collision-resistant names

The repository identifier is `styrene:git:v1:<digest>`, where `<digest>` is a canonical
base32 representation of BLAKE3-256 over a domain separator and the initial deterministic
CBOR bytes. Later identity updates do not alter it.

Changing the encoding or digest requires a new identifier-algorithm version. Verifiers do
not re-encode untrusted identity bytes before determining the identifier. They verify that
the supplied bytes are already canonical.

## Styrene Identity binding

The `styrene-rs` OpenSpec change `repository-signing-profile` owns the canonical Identity
ID, epoch-indexed repository-signing family, binding schema, signing frame, byte limits,
strict verification, and immutable corpus. `styrene-git` does not duplicate those contracts.

The Git core consumes `styrene-identity` with default features disabled and only the minimal
repository-signing feature enabled. It re-exports or adapts the authoritative Identity ID
and verified binding domain values. It maps typed Identity failures into repository errors
without exposing CBOR library types across the package boundary.

Bindings remain reusable across repositories because they contain no repository ID.
Repository delegate policy names Identity IDs, not transport destinations or public keys.

Identity verification proves that one Identity authority assigned one repository key at
one epoch. Repository prior state decides which binding is current. Every identity approval
and reference transition names an epoch that must match the selected binding exactly.
Historical bindings can verify historical state without authorizing new state. The core
represents current authority as a verified `SignerSelection`, which is distinct from binding
bytes supplied by a transfer. Reference verification compares the exact identity, repository
key, and epoch before it verifies the operation signature or imports transfer objects.

The current private `StyreneIdentity` and `SignerBinding` implementations are spike code.
They are removed before the profile is treated as shipped behavior. No compatibility reader
is added unless persisted or externally consumed spike bindings exist at migration time.

Committed manifests use a released Identity version or immutable Git revision, never a
sibling path. Git keeps a small copy of immutable positive and negative vectors with source
revision, profile version, and digest provenance. A separate external gate tests latest,
previous-supported, and Identity-main compatibility lanes.

## Identity transition history

The initial identity document is sequence zero. Every later identity transition commits to:

- Repository identifier
- Prior identity-transition identifier
- Next sequence
- Complete replacement identity document
- Distinct approvals from delegates authorized by the prior state

Authorization always uses the prior state. This prevents a proposed delegate from
self-authorizing admission and prevents a removed delegate from authorizing subsequent
updates. Transitions form one accepted history in this first slice. Competing valid
identity transitions are retained as evidence but require explicit resolution before
either becomes authoritative.

## Namespace and signed-reference model

Each publisher has one logical Git namespace under a repository's bare object database.
The physical layout may use Git namespaces or an equivalent ref transaction layout, but
the public contract is publisher ownership, not a path convention.

A signed reference transition contains:

- Format and signature-domain versions
- Repository identifier
- Publisher Styrene identity and repository signing-key epoch
- Parent transition identifier
- Monotonic sequence
- Complete sorted resulting reference map

Signing the parent, sequence, repository, publisher, and complete map prevents replay,
grafting, cross-repository transplantation, and partial-response ambiguity. Transition
identifiers use BLAKE3-256 over deterministic CBOR. Git object identifiers retain the
algorithm used by the underlying Git repository and are tagged with that algorithm in
signed data.

New bare repositories use Git SHA-256 object IDs by default. Existing SHA-1 repositories
remain readable through an explicit legacy compatibility mode that emits a warning when
opened.

## Quarantine and commit

Incoming data first enters transfer-scoped quarantine. Verification proceeds in this order:

1. Decode and canonical-format validation.
2. Transfer manifest digest and prerequisite validation.
3. Git object integrity and reachability validation.
4. Repository identity and signer-binding validation.
5. Identity and reference transition lineage validation.
6. Delegate and namespace authorization.
7. Canonical default-branch calculation.
8. Atomic object promotion and ref transaction.

Failure before the final transaction leaves accepted repository state unchanged. Duplicate
valid transitions and objects return an idempotent already-present outcome.

## Canonical branch

Only verified namespaces of current delegates count. A commit becomes the canonical
default-branch head when at least the identity threshold of distinct delegates reference
that exact commit from the configured default branch.

Automatic canonical movement is fast-forward only. Threshold agreement on a divergent or
ancestor commit is retained as visible delegate state but does not rewind the canonical
branch. An explicit governance operation for canonical rollback is outside this change.

## Local Git behavior

The URL forms are:

```text
styrene:///git/v1/<repository-digest>
styrene:///git/v1/<repository-digest>/publisher/<publisher-id>
```

The repository-only form fetches canonical refs. The publisher form fetches one verified
namespace. Push defaults to the authenticated caller's namespace and cannot target another
publisher. The empty URI authority is mandatory. Optional repeated `label` query values are
sorted, unique policy hints for daemon routing. They are not part of repository identity or
authorization, and they cannot name an address or force a carrier. Fragments and all other
query keys are invalid.

A local push transaction creates Git objects, computes the complete next reference map,
obtains a repository signature, and atomically records the transition. It then enqueues a
durable daemon publication intent before it reports success. Repository and publication
records can use separate storage transactions only if restart recovery deterministically
recreates a missing publication intent from committed repository state. Lack of a network
route does not fail the completed local push. Lack of signer access, local authorization,
or durable publication-intent recording does.

The `styrene-git-ipc` contract contains no caller identity because the authenticated IPC
session supplies that identity. A push request cannot name a publisher namespace. A
synchronization request names only a repository view and cannot select a peer, carrier,
route, or address. A committed push response returns a separate publication operation ID.
Clients resume durable operation observations by sequence after an IPC disconnect.

### Remote-helper command loop

Git starts `git-remote-styrene` with the remote name and URL. The executable validates the
complete URL before opening an IPC session, then runs the line-oriented Git remote-helper
protocol on standard input and standard output. Diagnostics use standard error and never
share the protocol stream.

The first implementation advertises only `fetch` and `push`. It handles `capabilities`,
`list`, `list for-push`, a blank-line-terminated batch of `fetch <object> <ref>` commands,
and a blank-line-terminated batch of `push [+]<source>:<destination>` commands. It does not
advertise `option`, `connect`, `stateless-connect`, or refspec rewriting. Unsupported or
malformed commands fail deterministically instead of being interpreted as Git revisions or
reference names.

`list` reads the canonical or publisher view selected by the URL. The daemon listing also
supplies an optional symbolic `HEAD` target so clone can select the repository's configured
default branch. `list for-push` is different: it reads a dedicated caller-owned push view.
It cannot reuse the canonical view or accept a publisher from the URL because the IPC
session, rather than request data, determines the writable namespace. The IPC contract
therefore has separate read-view and caller-owned push-list operations.

The helper accumulates each fetch batch into one bounded IPC fetch request. A Git-plumbing
adapter derives locally present prerequisites and installs the returned pack in the calling
repository only after the client has validated the response. The helper accumulates each
push batch into one atomic IPC push request. It resolves sources in the calling repository,
uses the caller-owned push listing for expected destination values, preserves force and
deletion semantics, and creates one pack for all non-deletion updates. A successful atomic
response emits `ok` for every destination. A rejection emits `error` for every affected
destination and does not report a partial success.

### Local Git IPC client boundary

The helper-facing client maps one typed `styrene-git-ipc` request to one typed response.
It generates a fresh request ID and enforces negotiated limits before writing. These limits
cover request size, pack size, references, wants, and haves. The client rejects a response
with the wrong version, request ID, body kind, or size. The initial helper is single-flight.
An unsolicited or mismatched response is a terminal session error. Read-only requests may
be retried only when a caller explicitly starts a new attempt. Push requests are never
retried automatically because an IPC failure can occur after the daemon commits them.

The client core receives an already-established authenticated Git service transport. It
does not discover a socket, infer caller identity from a path, assign a daemon opcode, or
duplicate the existing daemon's MessagePack envelope. A deterministic in-memory transport
tests request mapping and failures. The production `styrened` adapter owns authentication,
the control-socket envelope and opcode, dispatch, and conversion to that transport as part
of `GitService` composition. Owner-only socket permissions alone are not described as
cryptographic authentication.

Git subprocesses are behind a narrow adapter covering object-format discovery, revision
resolution, local prerequisite enumeration, push-pack creation, and fetched-pack
installation. Tests use deterministic fake implementations for command-loop cases. Real
Git process tests cover the adapter separately. The final black-box test uses the shipped
helper and authenticated daemon adapter. It does not use a production test-mode bypass.

## Replication contract

The repository protocol defines:

- Repository and publisher state summaries
- Explicit wants and prerequisites
- Transfer manifests
- Git pack or bundle payload references
- Signed transition payloads
- Deterministic verification and commit outcomes

`styrened` may carry a transfer over native RNS Resources, `styrene-content`, a direct
stream, a negotiated WireGuard tunnel, Yggdrasil, I2P, or offline media. Carrier identity
is evidence for operations and policy but never repository authorization.

## Failure and operation semantics

Repository mutation has one atomic outcome. Daemon synchronization has a separate durable
operation lifecycle with correlation, deadline, cancellation, retry, selected peer,
selected carrier, and terminal outcome. An IPC disconnect does not cancel daemon-owned
work or roll back a committed repository transaction.

Temporary transfers are bounded by size, time, and storage policy. No untrusted object is
made reachable from an accepted ref before verification and commit.

## Functional harness

The first functional backend is a test-only operator process over the production core,
store, and protocol crates. It creates real SHA-256 Git objects and signed transfers. It is
not a substitute for the later `styrened` and remote-helper acceptance backend.

Podman Compose starts three operators with separate named volumes and one scenario runner.
The runner uses an isolated test-control network to request actions and relay opaque transfer
bytes. Operators do not share repository storage, working directories, or private keys.

Scenarios use deterministic fixture seeds and predicate-based waits. They do not use sleeps
as correctness conditions. Every run writes a bounded JSON artifact with operator IDs,
repository ID, heads, transition outcomes, canonical decisions, and the scenario result.
Private fixture keys are never written to artifacts.

The initial smoke scenario proves three-party divergence, threshold convergence, retained
publisher state, duplicate delivery, and identical final state. Later backends reuse the
scenario contract for production daemon, remote-helper, carrier-fault, and crash-recovery
testing.

The full lab also co-deploys one `styrened` hub and one full node per Git operator. Each
daemon has private persistent identity and data volumes. A Git operator can access only its
matching owner-mode Unix socket; the legacy daemon RPC port is not published. The RNS TCP
backbone is isolated from the Git test control and synthetic carrier networks.

Until `GitService`, synchronization coordination, and the RNS Resource adapter exist, the
lab proves daemon IPC, discovery, and LXMF delivery independently and records the Git
transport adapter as unconnected. It must not relay Git transfer bytes over LXMF or claim
that co-deployment alone satisfies the repository replication contract.

Lab executables are built into signed local APK packages with Melange and assembled into
minimal, SBOM-producing images with APKO. The package build uses Wolfi's Rust 1.97 toolchain
and the checked-in Cargo lockfiles. Dockerfiles and Docker/BuildKit image assembly are not
part of the lab build path. Post-assembly verification requires the signed APK index and
package, OCI archive and normalized image metadata, expected runtime executables, and SPDX
2.3 documents that include the corresponding local package.

## Deferred decisions

- Encrypted private-repository object distribution and recipient revocation
- Identity-key revocation beyond monotonically increasing signer epochs
- Explicit canonical rollback governance
- Collaboration-object schemas and `styrene-work` projection
- Large-file extension protocols
- Radicle wire compatibility or migration
