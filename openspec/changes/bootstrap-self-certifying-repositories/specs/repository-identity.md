# Repository Identity - Delta Spec

## ADDED Requirements

### Requirement: Initial identity deterministically defines the repository identifier

Every repository has a versioned identity document containing its project metadata,
default branch, visibility, delegate set, and delegate threshold. The repository
identifier is derived from the deterministic canonical encoding of the initial identity
document and remains unchanged for the life of that identity history.

#### Scenario: Independent repository initialization
Given two implementations receive the same valid initial identity fields.
When each implementation creates the canonical identity document.
Then both implementations produce identical canonical bytes and the same repository identifier.
And the identifier does not contain a host, route, or transport address.

#### Scenario: Identity metadata changes
Given a repository has an accepted identity history and stable repository identifier.
When an authorized update changes the name or description.
Then the repository identifier remains unchanged.
And the updated metadata is associated with the existing repository identity history.

#### Scenario: Non-canonical identity encoding
Given identity bytes do not satisfy the declared deterministic encoding profile.
When a verifier loads the repository identity.
Then verification fails before repository state is accepted.
And the verifier does not silently normalize the untrusted bytes into a different identity.

### Requirement: Repository signers are bound to Styrene identities

A repository authority key must have a verifiable, domain-separated binding. The binding
names one canonical Styrene identity under the released Styrene
Identity repository-signing profile. `styrene-git` consumes the authoritative Identity ID,
key purpose, canonical binding bytes, signature frame, and verification result. It does not
define a parallel identity identifier or binding profile. Repository signatures are distinct
from ordinary Git commit signatures and transport authentication.

#### Scenario: Valid repository signer binding
Given a repository signing key has a binding signed by its canonical Styrene identity.
When Styrene Identity verifies the profile, identity, purpose, epoch, and key.
Then repository operations signed by that key are attributed to the bound Styrene identity.

#### Scenario: Transport key presented as repository authority
Given a peer has established an authenticated RNS link or WireGuard session.
When the peer presents repository state without a valid repository signer binding.
Then the repository state is rejected as unauthorized.
And transport authentication does not grant repository authority.

#### Scenario: Binding from another identity
Given a repository update names delegate A as its author.
When its repository signing key is bound to Styrene identity B.
Then verification rejects the update.

#### Scenario: Ordinary Git signing key presented as repository authority
Given a valid Git commit signature from the canonical Identity key.
When the signer has no valid repository-signing profile binding.
Then the repository update is rejected as unauthorized.

#### Scenario: Git-local binding format is presented
Given bytes satisfy an obsolete or private Git-local signer-binding format.
When repository authorization verifies the bytes.
Then verification rejects them unless an explicit released compatibility profile applies.

### Requirement: Repository operations select bindings under accepted epoch policy

A cryptographically valid Identity binding proves assignment of a repository key at one
epoch but does not make that epoch current for repository state. Every signed repository
operation names its binding epoch. Verification selects the accepted binding under prior
repository state and requires exact identity, key, and epoch agreement. An identity-transition
approval uses a versioned signing frame that commits to the approving identity and epoch.

#### Scenario: Current epoch signs next repository state
Given prior repository state selects identity A's binding at epoch one.
When A signs the next operation with that key and names epoch one.
Then the operation is eligible for repository policy and lineage verification.

#### Scenario: Stale binding signs new repository state
Given prior repository state selects identity A's binding at epoch one.
When an epoch-zero key signs a new operation that names epoch zero.
Then the operation is rejected under prior repository state.
And the epoch-zero binding remains usable only for retained historical verification.

#### Scenario: Claimed epoch differs from supplied binding
Given an operation names epoch one and the supplied binding is valid at epoch zero.
When repository authorization verifies the operation.
Then the operation is rejected before its signature can authorize repository state.

#### Scenario: Maximum epoch is verified without increment
Given prior repository state selects a valid binding at epoch `u32::MAX`.
When an operation names that epoch.
Then verification does not increment or wrap the epoch.

#### Scenario: Approval metadata is relabeled
Given the same repository public key appears in bindings for two identities or epochs.
When an approval signature is moved to the other identity or epoch.
Then the approval signature does not verify.

#### Scenario: Identity sequence is exhausted
Given accepted identity state has sequence `u64::MAX`.
When a caller proposes or verifies another identity transition.
Then the operation fails without increment, panic, wraparound, signature, or state change.

### Requirement: Git independently verifies immutable Identity conformance vectors

`styrene-git` retains a small provenance-recorded copy of released positive and negative
repository-signing vectors. Normal Git tests verify those bytes independently of the
Identity vector generator. A separate two-checkout gate tests supported Identity revisions
without committing a sibling path dependency.

#### Scenario: Released positive binding vector
Given a released Identity profile vector and its recorded provenance.
When the Git conformance test decodes and verifies it.
Then the Identity ID, binding fields, frame, signature, and digest match exactly.

#### Scenario: Released negative binding vector
Given a released malformed or mutated binding vector with an expected rejection class.
When the Git conformance test verifies it.
Then verification fails with the mapped stable rejection class.

#### Scenario: Latest Identity compatibility run
Given the external conformance gate checks out exact Identity and Git revisions.
When the Identity corpus and Git consumer tests run.
Then the result records both revisions and any failing property-test seed.
And neither repository is added to the other's ordinary workspace.

### Requirement: Delegate policy is valid and explicit

Every accepted identity state contains unique Styrene delegates. Its nonzero threshold
does not exceed the number of delegates.

#### Scenario: Valid delegate threshold
Given an identity document names three unique delegates and threshold two.
When the identity state is validated.
Then the delegate policy is accepted as requiring two authorized approvals.

#### Scenario: Impossible threshold
Given an identity document names two delegates and threshold three.
When the identity state is validated.
Then the identity state is rejected.

#### Scenario: Duplicate delegate
Given an identity document repeats one Styrene identity in its delegate set.
When the identity state is validated.
Then the identity state is rejected rather than counting the identity twice.

### Requirement: Identity updates form an authorized transition history

An identity update names its repository, prior accepted identity state, monotonic
sequence, and proposed replacement document. Authorization is evaluated using the
delegate policy in the prior state, and each approving delegate is counted at most once.

#### Scenario: Threshold-authorized delegate change
Given the current identity state requires two of three delegates.
When two distinct current delegates approve a transition that replaces one delegate.
Then the new identity state is accepted.
And future updates are evaluated against the new delegate policy.

#### Scenario: New delegate self-authorizes admission
Given identity B is not a delegate in the current identity state.
When B signs an admission update without satisfying the current threshold.
Then the update is rejected.

#### Scenario: Identity transition replay
Given an identity transition sequence has already been accepted.
When the same transition or an earlier transition is presented as the next state.
Then the transition is rejected.
And the accepted identity head remains unchanged.

#### Scenario: Cross-repository identity transition
Given a valid signed identity transition belongs to repository A.
When it is presented as an update to repository B.
Then verification rejects it because the repository context does not match.
