# Repository State - Delta Spec

## ADDED Requirements

### Requirement: Each publisher exclusively controls one authenticated namespace

Repository storage separates Git references by publisher Styrene identity while sharing
the underlying Git object database. A namespace update is accepted only when signed by a
repository key bound to that namespace identity.

#### Scenario: Publisher advances its branch
Given publisher A has a verified namespace in a repository
When A publishes a valid signed transition advancing `refs/heads/main`
Then only A's namespace is updated
And references in every other publisher namespace remain unchanged

#### Scenario: Publisher writes another namespace
Given publisher A signs a transition naming publisher B's namespace
When the transition is verified
Then the transition is rejected
And B's accepted references remain unchanged

#### Scenario: Shared Git object
Given publisher A and publisher B reference the same Git object
When both namespaces are stored
Then the object may be stored once in the shared object database
And each publisher's reference ownership remains independently verifiable

### Requirement: Published references use replay-resistant signed transitions

Each signed reference transition commits to the repository identifier, publisher identity,
prior accepted transition, monotonic sequence, and complete resulting reference map. A
transition cannot be validly transplanted to another repository, publisher, or history.
The signature uses an explicit versioned reference-transition frame.

#### Scenario: Valid next reference transition
Given a publisher has an accepted reference transition at sequence seven
When the publisher signs sequence eight with the accepted transition as parent
Then the transition is eligible for object and policy verification

#### Scenario: Previously signed state is replayed
Given a publisher has advanced from sequence seven to sequence eight
When an untrusted peer presents the signed sequence-seven state as a new update
Then the update is rejected as stale
And sequence eight remains the accepted state

#### Scenario: Signed state is grafted onto another history
Given a signed reference map was produced with parent transition X
When it is presented with parent transition Y
Then signature or transition verification fails

#### Scenario: Reference transition omits an existing reference
Given a publisher's accepted state contains two references
When a validly signed next transition contains the complete resulting map with one reference removed
Then the removal is explicit and verifiable
And absence cannot be confused with a partial network response

#### Scenario: Signature is moved from another domain
Given a valid signature was created for another Styrene operation or signing-frame version
When it is attached to a reference transition
Then reference-transition signature verification fails

#### Scenario: Reference sequence is exhausted
Given accepted publisher state has sequence `u64::MAX`
When a caller constructs or verifies another reference transition
Then the operation fails without increment, panic, wraparound, signature, or state change

### Requirement: Untrusted objects are quarantined until verification completes

Objects and transitions received from another peer do not become visible through accepted
references until Git object integrity, repository context, signer binding, transition
lineage, reference targets, and applicable policy have all been verified.

#### Scenario: Transfer contains a corrupt object
Given an incoming transfer contains a reference to an object whose bytes do not match its Git object identifier
When transfer verification runs
Then the transfer is rejected
And no accepted namespace reference points to any object from that transfer

#### Scenario: Transfer is interrupted
Given an incoming transfer ends before all declared objects arrive
When the transfer is finalized
Then finalization fails as incomplete
And previously accepted repository state is unchanged

#### Scenario: Valid transfer is committed
Given all declared objects, signatures, transitions, and policies verify
When the transfer is finalized
Then objects and accepted references become visible atomically

### Requirement: Canonical default branch is derived from delegate agreement

The canonical default branch is the commit referenced by at least the active identity
threshold of distinct delegates in their verified namespaces. Canonical derivation is
deterministic for the same verified repository state.

#### Scenario: Delegate threshold agrees
Given a repository has three delegates, threshold two, and default branch `main`
When two delegates' verified `main` references point to the same descendant commit
Then that commit becomes the canonical `main` head

#### Scenario: Delegates disagree below threshold
Given no commit is referenced by enough delegates to satisfy the threshold
When canonical state is calculated
Then the previously accepted canonical head is retained
And no delegate namespace is discarded or overwritten

#### Scenario: Candidate rewinds canonical history
Given a canonical default-branch head is already accepted
When a threshold of delegates references a commit that is not a descendant of that head
Then automatic canonical advancement is refused
And the divergent delegate references remain available for inspection

#### Scenario: Non-delegate agreement
Given multiple non-delegate publishers reference the same commit
When canonical state is calculated
Then their references do not count toward the delegate threshold

### Requirement: Functional scenarios isolate operator authority and storage

The functional harness runs each operator in a separate process with a distinct Identity,
repository-signing key, working directory, and repository store. Operators exchange only
carrier-neutral transfer bytes. Test control cannot grant repository authority.

#### Scenario: Three operators diverge and converge
Given Alice, Bob, and Carol are delegates with threshold two.
When Bob and Carol publish different descendants and Alice later selects Bob's descendant.
Then Bob's descendant becomes canonical on every synchronized operator.
And Carol's divergent publisher reference remains available.

#### Scenario: Operator storage is isolated
Given three functional operators share no repository or identity volume.
When one operator commits and another receives its transfer.
Then the receiver advances only after transfer verification and application.

#### Scenario: Duplicate functional delivery
Given one signed transfer has already been applied.
When the harness delivers the same bytes again.
Then the receiver reports an already-present outcome.
And accepted repository state does not advance twice.

#### Scenario: Operator process restarts after commit
Given an operator has committed signed publisher transitions to its private repository volume.
When its process image is replaced and volatile harness context is reconstructed.
Then accepted publisher refs reopen from the repository store.
And canonical state derives to the same head.
And redelivery of the committed transition reports an already-present outcome.
