# Bootstrap Self-Certifying Repositories Tasks

## Delivery Roadmap

The task numbers group work by domain. They do not define implementation order. Use the
phases and exit gates below to select the next work.

| Phase | Purpose | Work | Exit gate |
| --- | --- | --- | --- |
| 0 | Freeze local contracts and record current security evidence | 2.8, 2.9, 3.4, 3.7, 4.4a, 5.2a, 5.3a, 5.4 | G0: frozen IPC, signature, and Git-plumbing contracts |
| 1 | Complete the daemon-independent helper | 3.5, 3.8, 3.9, 3.6, 3.10a, 3.10b | G1: deterministic helper-core acceptance |
| 2 | Replace spike Identity authority | 1.4, 1.7, 1.8, 1.10, 1.11, 1.12 | GI: released Identity profile conformance |
| 3 | Deliver authenticated local Git and offline push | 3.2a, 3.2b, 3.11a, 3.11b, 3.12a, 3.12b, 4.4b, 3.13a, 3.13b, 3.14 | GD: authenticated Git black-box acceptance |
| 4 | Make synchronization durable with a test carrier | 4.4c, 4.5a, 4.5b, 4.5c, 4.8a | GS1: restart-safe synchronization semantics |
| 5 | Connect the production RNS carrier | 4.6a, 4.6b, 4.8b | GS2: carrier-neutral production replication |
| 6 | Complete security and archival validation | 5.1, 5.2b, 5.2c, 5.3b, 5.5 | GV: all specs and immutable revisions reconciled |

Phase 2 runs in parallel with phases 0 and 1. Phase 3 can start only after G0 and GI pass.
Phase 4 can start after the authenticated service boundary and core idempotency are stable.
Phase 5 can start only after GS1 passes. Phase 6 closes work that runs continuously in the
earlier phases.

### G0: Frozen local contracts

- Read listings and caller-owned push listings are separate IPC operations.
- A read listing can identify a verified symbolic `HEAD`.
- Push requests contain no caller or target publisher identity.
- Versions, request correlation, stable errors, and all negotiated limits are fixed.
- No daemon envelope, socket, peer, route, carrier, or RNS type enters a Git crate contract.
- The Git-plumbing adapter has bounded inputs and identifies every subprocess failure.

### G1: Helper-core acceptance

- The command loop handles capabilities, both list forms, batched fetch, and batched push.
- Deterministic tests cover force, deletion, limits, malformed input, EOF, and response mismatch.
- An IPC failure after a push produces an indeterminate result and no automatic retry.
- Real Git process tests cover pack creation, pack installation, and supported object formats.
- The helper core uses only an injected authenticated transport and has no network authority.

### GI: Identity authority

- `styrene-rs` releases the repository-signing profile at an immutable revision or version.
- Git disables default Identity features and enables only the required signing feature.
- Git-local Identity IDs, purposes, bindings, and binding verification formats are removed.
- Positive and negative vectors record source revision, profile version, and digest provenance.
- Latest, previous-supported, and Identity-main gates record both repository revisions.
- Identity approval signatures commit to signer identity and epoch under a versioned frame.
- Identity sequence overflow is rejected without signing or hashing wrapped state.

### GD: Authenticated local Git

- The daemon authenticates a session principal before it evaluates repository RBAC.
- Authorization fails before signer access, object promotion, or reference mutation.
- Push success means accepted repository state and publication intent are both recoverable.
- Network reachability does not affect a committed local push result.
- The black-box test uses the installed helper and shipped daemon adapter without a test bypass.

### GS1: Durable synchronization

- Repository commit state and daemon operation state remain separate and correlated.
- Cancellation cannot roll back committed repository state.
- Retry and concurrent delivery cannot duplicate a committed transition, job, or canonical event.
- Terminal operation state and resumable observations survive daemon restart and IPC disconnect.
- The gate passes with a deterministic test carrier before production carrier integration.

### GS2: Production carrier neutrality

- The RNS adapter uses the existing daemon transport boundary without leaking RNS types.
- Test and RNS carriers produce the same repository result for the same transfer bytes.
- Carrier authentication remains evidence and cannot authorize repository state.
- Functional evidence identifies Git replication as connected and does not carry Git data over LXMF.

### GV: Archival readiness

- Both workspaces pass formatting, warning-denied Clippy, unit, property, integration, and black-box tests.
- Malformed, oversized, corrupt, interrupted, and signature-fuzz corpora pass at recorded revisions.
- Every signed domain has a final context and canonical-encoding review.
- Heartwood provenance and license obligations are recorded.
- Every scenario in all four delta specifications maps to passing evidence or an explicit exclusion.

## 1. Repository Identity And Fixtures
<!-- specs: repository-identity -->

- [x] 1.1 Create the `styrene-git-core` crate with typed repository IDs, Styrene identity IDs, signer bindings, identity documents, delegate policies, and transition errors
- [x] 1.2 Implement RFC 8949 deterministic CBOR encoding and reject non-canonical signed input
- [x] 1.3 Implement versioned `styrene:git:v1` BLAKE3-256 identifier derivation with golden byte and identifier fixtures
- [ ] 1.4 Complete and release the `styrene-rs` `repository-signing-profile` change with read-only positive and negative vectors. This external task gates GI
- [x] 1.5 Implement prior-state threshold authorization, unique-approval counting, sequence checks, and repository-context checks for identity transitions
- [x] 1.6 Add tests for impossible thresholds, duplicate delegates, unauthorized admission, replay, reordering, and cross-repository transitions
- [ ] 1.7 Depend on the minimal Identity repository-signing feature and remove Git-local Identity ID and binding implementations
- [ ] 1.8 Import provenance-recorded immutable binding vectors and test canonical, malformed, cross-purpose, and cross-identity outcomes
- [x] 1.9 Implement prior-state current-epoch selection and generated stale, future, rotated, substituted, and maximum-epoch cases
- [ ] 1.10 Add latest, previous-supported, and Identity-main exact-revision conformance lanes without committed sibling path dependencies, and record both revisions and failing property seeds
- [ ] 1.11 Use a versioned approval signing frame during Identity migration. Commit to signer identity and epoch, and reject relabeling when a repository key is reused
- [x] 1.12 Reject identity transition construction and verification at maximum sequence without increment, panic, or wraparound

## 2. Namespaces And Signed References
<!-- specs: repository-state -->

- [x] 2.1 Create `styrene-git-store` with bare repository creation, repository lookup, shared object storage, and publisher namespace transactions
- [x] 2.2 Define deterministic signed reference transitions containing repository, publisher, key epoch, parent, sequence, and the complete sorted reference map
- [x] 2.3 Add replay, graft, cross-repository, cross-publisher, partial-map, and stale-sequence rejection tests
- [x] 2.4 Implement transfer-scoped Git object quarantine with object-ID, target-existence, and reachability verification
- [x] 2.5 Atomically promote verified objects and update accepted namespace refs without exposing partial state
- [x] 2.6 Implement deterministic threshold canonicalization with fast-forward-only automatic advancement and divergent-state retention
- [x] 2.7 Add multi-delegate tests for agreement, disagreement, non-delegate votes, duplicate votes, and rewind refusal
- [x] 2.8 Add an explicit versioned reference-transition signing frame with golden bytes and cross-domain signature-substitution tests
- [x] 2.9 Reject reference transition construction and verification at maximum sequence without increment, panic, or wraparound

## 3. Local Git And Daemon Boundary
<!-- specs: local-git-integration -->

- [x] 3.1 Define `styrene-git-ipc` request, response, stable error, capability, operation, and observation types for repository operations
- [ ] 3.2a Define the `styrened` authenticated Git session principal, production control-socket envelope and opcode, connection limits, and helper connector without treating socket permissions as authentication
- [ ] 3.2b Compose `GitService` in `styrened` and apply repository RBAC before signer access, object promotion, or reference mutation
- [x] 3.3 Implement strict carrier-neutral `git-remote-styrene` URL parsing, canonical rendering, synchronization-hint mapping, and malformed-input tests
- [x] 3.4 Extend `styrene-git-ipc` with an optional symbolic HEAD in read listings and a dedicated caller-owned push-list request and response that contain no publisher identity
- [x] 3.5 Implement the single-flight helper IPC client over an injected authenticated transport. Enforce fresh request IDs, negotiated bounds, typed responses, and no automatic push retry
- [x] 3.6 Add an installable `git-remote-styrene` binary and a line-oriented command loop for helper invocation, capabilities, `list`, `list for-push`, batch termination, stable stderr diagnostics, and unsupported input
- [x] 3.7 Implement the Git-plumbing adapter for object-format discovery, revision resolution, bounded local prerequisite enumeration, push-pack creation, and fetched-pack installation
- [x] 3.8 Map one fetch batch to one bounded IPC request with unique wants and haves, correlated response validation, and pack installation after validation
- [x] 3.9 Map one push batch, including force and deletion, to one atomic caller-owned IPC request using refreshed destination expectations and one bounded pack
- [x] 3.10a Add deterministic in-memory transport and fake Git-plumbing tests. Cover all helper commands, force, deletion, limits, malformed input, response mismatch, EOF, and indeterminate pushes
- [x] 3.10b Add real Git subprocess tests for SHA-256 and supported SHA-1 repositories, revision resolution, bounded pack creation, fetched-pack installation, and subprocess failure reporting
- [ ] 3.11a Implement canonical and publisher read listings and fetches in `GitService`, including verified symbolic `HEAD`
- [ ] 3.11b Implement caller-owned push listing and namespace mutation without accepting a publisher identity from the request
- [ ] 3.12a Commit signed local reference transitions and object promotion as one recoverable repository transaction
- [ ] 3.12b Record publication intent durably before reporting push success, recover interrupted scheduling, and deduplicate publication work independently of network reachability
- [ ] 3.13a Add helper and connector integration tests for daemon unavailability, malformed URLs, protocol mismatch, limit failures, and indeterminate push responses
- [ ] 3.13b Add daemon integration tests for offline push, signer unavailability, unauthenticated and unauthorized callers, cross-namespace rejection, and publication-intent recovery
- [ ] 3.14 Add a black-box Git test for initialize, push, clone, fetch, and checkout. Use only `styrene://` and the shipped authenticated daemon and helper interfaces

## 4. Carrier-Neutral Replication
<!-- specs: replication-contract, repository-state -->

- [x] 4.1 Create `styrene-git-protocol` with versioned state summaries, wants, prerequisites, transfer manifests, payload descriptors, and typed outcomes
- [x] 4.2 Encode protocol values as deterministic CBOR and add golden fixtures independent of any Styrene carrier
- [x] 4.3 Implement Git pack or bundle export and import with explicit prerequisites, BLAKE3 integrity, bounded sizes, and quarantine handoff
- [x] 4.4a Make duplicate transition application and object promotion idempotent in core and store, including concurrent commit linearization
- [ ] 4.4b Deduplicate durable publication jobs and canonical events across local push recovery and duplicate transfer application
- [ ] 4.4c Prove concurrent synchronization and publication attempts converge to one logical transition, publication job, and canonical event
- [ ] 4.5a Implement the durable `styrened` synchronization operation state machine, correlation, terminal outcomes, and repository port
- [ ] 4.5b Add seeding policy, peer and carrier selection, deadlines, cancellation, bounded retries, and commit-aware retry suppression
- [ ] 4.5c Persist ordered observations and resume them by sequence after IPC disconnect or daemon restart
- [ ] 4.6a Define and test the daemon carrier adapter boundary with a deterministic test carrier and no carrier types in Git crates
- [ ] 4.6b Implement the RNS Link/Resource adapter using existing `MeshTransport` and connect it to the functional lab without carrying Git transfers over LXMF
- [x] 4.7 Add tests proving the same transfer has identical repository results through two test carriers and that carrier authentication cannot authorize repository state
- [ ] 4.8a Add test-carrier restart, cancellation, retry, duplicate-delivery, and IPC-disconnect tests that preserve committed state and terminal outcomes
- [ ] 4.8b Repeat restart, retry, disconnect, and duplicate-delivery acceptance through the production RNS adapter

## 5. Validation And Security Review
<!-- specs: repository-identity, repository-state, local-git-integration, replication-contract -->

- [ ] 5.1 Run formatting, warning-denied Clippy, unit, property, integration, and black-box Git tests for both workspaces
- [x] 5.2a Run malformed-CBOR, oversized-transfer, corrupt-object, and interrupted-transfer corpora against existing decode, pack, and quarantine boundaries
- [ ] 5.2b Add authoritative Identity binding and repository-signature mutations after GI passes
- [ ] 5.2c Add coordinator cancellation, interruption, retry, and restart corpora after GS1 passes
- [x] 5.3a Inventory every current signed domain and its version, repository context, publisher context, parent linkage, sequence, and canonical encoding
- [ ] 5.3b Repeat the signed-domain review after all mutation paths exist and record final evidence
- [x] 5.4 Record Heartwood provenance, identify whether any code was imported, and satisfy the selected MIT or Apache-2.0 obligations before later implementation copies code
- [ ] 5.5 Compare every implemented behavior and failure outcome against all four delta specifications before requesting archival
- [x] 5.6 Maintain an immutable-revision Heartwood behavioral parity inventory with explicit adopt, defer, skip, evidence, and gate decisions
- [x] 5.7 Add an isolated three-operator functional harness with deterministic scenario artifacts and production-library state transitions
- [x] 5.8 Add a Podman Compose smoke gate for multi-party divergence, convergence, retained publisher refs, and duplicate delivery
- [x] 5.9 Extend the multi-operator gate with corrupt, truncated, missing-prerequisite, unchanged-state, and valid-redelivery outcomes
- [x] 5.10 Prove an isolated operator process restart reopens committed refs, rederives canonical state, preserves idempotency, and passes fsck
- [x] 5.11 Co-deploy a persistent styrened hub and per-operator nodes, prove IPC discovery and LXMF delivery, and record the Git adapter as unconnected
