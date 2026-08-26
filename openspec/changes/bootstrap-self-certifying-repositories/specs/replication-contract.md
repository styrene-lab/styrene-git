# Replication Contract - Delta Spec

## ADDED Requirements

### Requirement: Repository replication is independent of network carrier

The repository layer expresses signed state offers, wants, and verifiable object transfers
without importing RNS, WireGuard, Yggdrasil, I2P, socket, route, or tunnel types. A transfer
has identical repository meaning regardless of the carrier selected by `styrened`.

#### Scenario: Same update arrives over different carriers
Given two transfers contain the same signed repository transition and Git objects
When one arrives through an RNS Resource and one through a direct stream
Then both produce the same verified repository result
And carrier metadata does not change repository authority

#### Scenario: Carrier authenticates an unauthorized peer
Given `styrened` authenticates a network peer and successfully receives its transfer
When repository signatures or policy do not authorize the contained update
Then the repository layer rejects the update
And carrier authentication is retained only as transport evidence

### Requirement: Replication transfers are self-describing and integrity checked

A transfer identifies its format version, repository, publisher states, prerequisites,
included objects or bundle, and integrity digest. The receiver can determine whether it
has the prerequisites before committing any state.

#### Scenario: Receiver has all prerequisites
Given a transfer declares a prerequisite state already accepted by the receiver
When all included data verifies
Then the receiver can atomically apply the transfer

#### Scenario: Receiver lacks a prerequisite
Given a transfer depends on an object or transition the receiver does not possess
When the receiver validates the transfer manifest
Then it reports the missing prerequisite
And it does not partially advance accepted state

#### Scenario: Transfer payload is modified
Given transfer bytes differ from the signed manifest or integrity digest
When the receiver verifies the transfer
Then the transfer is rejected

#### Scenario: Transfer delivery is truncated
Given a carrier stops delivering a transfer before its canonical envelope is complete
When the receiver decodes the incomplete bytes
Then the transfer is rejected
And accepted repository state does not change

### Requirement: Replication application is idempotent

Receiving the same valid transfer more than once has the same accepted result as receiving
it once and does not duplicate transitions, objects, publication jobs, or canonical events.

#### Scenario: Valid transfer is delivered twice
Given a valid transfer has already been committed
When the identical transfer is delivered again
Then the second application reports an already-present outcome
And accepted repository and canonical state do not change

#### Scenario: Concurrent duplicate delivery
Given two carriers concurrently deliver the same valid transition
When both verification attempts complete
Then at most one commits the transition
And both callers observe the same final accepted state

### Requirement: Styrened owns synchronization policy and operation lifecycle

The repository layer supplies deterministic validation and mutation operations, while
`styrened` owns peer selection, carrier selection, deadlines, cancellation, retries,
seeding policy, and correlated operator observations.

#### Scenario: Preferred carrier fails
Given a synchronization job has multiple permitted carriers
When the preferred carrier fails before repository commit
Then `styrened` may retry through another carrier under its policy
And the repository layer receives the same carrier-neutral transfer contract

#### Scenario: Synchronization is cancelled
Given a synchronization operation has not committed repository state
When `styrened` cancels the operation
Then temporary transfer data is eligible for cleanup
And accepted repository state remains unchanged

#### Scenario: Transfer commits before observation delivery
Given a transfer has atomically committed but an IPC client disconnects before receiving the event
When the client later reconnects and queries operation and repository state
Then the daemon reports the committed terminal outcome
And the transfer is not repeated solely because the earlier observation was lost

### Requirement: Functional deployment includes an isolated Styrene backbone

The full functional lab co-deploys a `styrened` hub and one full node beside each Git
operator. Daemon identities, data, IPC sockets, and Git repository stores use distinct
persistent volumes. The lab does not publish legacy daemon RPC ports.

#### Scenario: Backbone is operational beside Git operators
Given the hub, three daemons, and three Git operators are running in the full lab.
When the acceptance sidecar queries each private daemon socket and waits for announcements.
Then every daemon reports status.
And the hub discovers Alice, Bob, and Carol through RNS TCP interfaces.

#### Scenario: Backbone carries an authenticated application delivery
Given Alice and Bob have discovered one another through the hub.
When Alice sends a correlated LXMF message to Bob's announced destination.
Then Bob receives the correlated message before the bounded deadline.

#### Scenario: Git transport adapter is not yet implemented
Given the Styrene backbone and Git operators are co-deployed.
When no `GitService` or RNS Resource adapter is installed.
Then Git transfer bytes remain on the test-only carrier.
And acceptance evidence identifies the Git transport adapter as unconnected.
