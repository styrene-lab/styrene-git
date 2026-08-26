# Local Git Integration - Delta Spec

## ADDED Requirements

### Requirement: Git accesses Styrene repositories through a remote helper

The Git remote helper recognizes versioned `styrene:///git/v1/` repository URLs and maps
Git fetch and push operations to typed local daemon requests. A URL identifies a repository
and may identify a publisher view. Optional `label` query values are non-authoritative daemon
policy hints. A Git URL never identifies a network host, peer, carrier, route, or address.

#### Scenario: Fetch canonical repository view
Given a local working copy has remote URL `styrene:///git/v1/<repository-digest>`
When Git requests the remote references
Then the helper returns the locally verified canonical repository view
And it does not select a remote network peer

#### Scenario: Fetch publisher repository view
Given a local working copy has remote URL `styrene:///git/v1/<repository-digest>/publisher/<publisher-id>`
When Git requests the remote references
Then the helper returns the locally verified references for that publisher namespace

#### Scenario: Routing labels do not change repository authority
Given a repository URL contains sorted unique `label` query values
When the helper requests synchronization from the daemon
Then the labels are passed as non-authoritative policy hints
And they do not change the repository identifier, publisher view, or accepted signatures

#### Scenario: Malformed Styrene URL
Given a remote URL has an invalid repository or publisher identifier
When Git invokes the remote helper
Then the operation fails with a stable validation error
And no repository storage or network operation begins

### Requirement: The executable implements bounded Git remote-helper commands

The installed `git-remote-styrene` executable implements the line-oriented Git
remote-helper protocol for capabilities, reference listing, batched fetch, and batched
push. It keeps protocol output separate from diagnostics and does not advertise commands
that it does not implement.

#### Scenario: Git discovers helper capabilities
Given Git invokes `git-remote-styrene` with a valid Styrene repository URL
When Git sends the `capabilities` command
Then the helper advertises `fetch` and `push`
And it terminates the capability list with a blank line

#### Scenario: Unsupported or malformed helper command
Given the helper receives a command outside its advertised grammar or a malformed batch
When it parses the command
Then it fails with a stable diagnostic and non-zero process status
And it does not reinterpret command fields as arbitrary Git command-line arguments

#### Scenario: Diagnostics do not corrupt the Git protocol
Given an IPC, URL, or Git-plumbing operation fails
When the helper reports the failure
Then protocol responses remain on standard output
And human-readable diagnostics are written only to standard error

### Requirement: Read and push listings use distinct authority contexts

The helper maps `list` to the URL-selected canonical or publisher read view. It maps
`list for-push` to a dedicated caller-owned view whose publisher is resolved from the
authenticated local session and cannot be supplied in the request.

#### Scenario: List read view for clone
Given a valid canonical or publisher repository URL
When Git sends `list`
Then the helper returns the verified references for that URL view
And it advertises symbolic `HEAD` when the daemon identifies a verified default branch

#### Scenario: List caller-owned push view
Given the authenticated caller owns publisher A's namespace
And the URL selects the canonical view or publisher B's read view
When Git sends `list for-push`
Then the helper requests publisher A's writable references without naming A in the request
And it does not use the URL view as push authority

### Requirement: Fetch commands form one bounded request

The helper collects one blank-line-terminated fetch command batch, validates its object IDs
and reference names against the calling repository's object format, and sends one bounded
typed fetch request. It installs returned objects only after validating the correlated IPC
response.

#### Scenario: Fetch several advertised references
Given Git sends several valid fetch commands followed by a blank line
When the helper processes the batch
Then it sends one request containing the unique wanted objects and bounded local prerequisites
And it installs the returned pack through Git plumbing before completing the batch

#### Scenario: Fetch exceeds a negotiated limit
Given the fetch batch or returned pack exceeds a daemon-advertised limit
When the helper validates the operation
Then it fails before an oversized request or pack is accepted
And no fetched object is installed by that operation

### Requirement: Push commands form one atomic caller-owned update

The helper collects one blank-line-terminated push batch, supports force and deletion
syntax, resolves source revisions through Git, obtains expected destination values from the
caller-owned push listing, and sends one typed push request with one bounded pack. The push
request contains no caller or target publisher identity.

#### Scenario: Push an atomic update batch
Given Git supplies valid create, update, force, or deletion commands in one push batch
When the daemon commits the complete batch
Then the helper reports `ok` for every destination
And network publication remains a separate daemon-owned operation

#### Scenario: Atomic push is rejected
Given any update in a push batch is unauthorized, stale, malformed, or cannot be signed
When the daemon rejects the batch
Then the helper reports an error for every affected destination
And it does not report a partial successful push

#### Scenario: Push response is lost after commit
Given the helper sends a push and the IPC session fails before a response is validated
When the helper reports the indeterminate result
Then it does not automatically resend the mutating request
And a later Git invocation must refresh the caller-owned push listing before another push

### Requirement: The helper IPC client fails closed at the session boundary

The helper-facing client uses an already-established authenticated Git service transport.
For each single-flight request it enforces negotiated limits and validates protocol version,
request correlation, response kind, and response size. Production socket authentication,
framing, opcode assignment, and dispatch are owned by the `styrened` Git service adapter.

#### Scenario: Correlated typed response
Given an authenticated local Git service transport is established
When the helper sends a request and receives the expected response with the same request ID
Then the client returns the typed response to the command loop

#### Scenario: Mismatched or unsolicited response
Given the helper has one request in flight
When the transport returns another request ID, an unexpected response body, or an unsupported version
Then the client closes or discards the failed session and returns a stable protocol error
And it does not apply Git objects or report a successful push

#### Scenario: Production transport is not available
Given no authenticated `styrened` Git service transport can be established
When the helper needs repository data
Then it fails without treating owner-only socket permissions as cryptographic authentication
And it does not fall back to a carrier or test-only daemon mode

### Requirement: Local pushes remain available offline

A push to the caller's own namespace is committed to local repository storage and signed
without requiring a network connection. Network publication is subsequent daemon-owned
work and does not determine whether the local push succeeds.

#### Scenario: Push while disconnected
Given `styrened` is running with local storage and identity access but has no network route
When Git pushes a valid branch update to the caller's Styrene namespace
Then the signed local reference transition is committed
And publication is recorded as pending without failing the Git push

#### Scenario: Process stops between repository commit and publication scheduling
Given a signed local reference transition has committed
And the process stops before ordinary publication scheduling completes
When `styrened` recovers the repository and publication state
Then exactly one pending publication intent exists for the committed transition
And the transition is not rolled back or signed again

#### Scenario: Push targets another publisher
Given the caller controls publisher A's repository key
When Git attempts to push directly into publisher B's namespace
Then the daemon rejects the push as unauthorized
And B's namespace remains unchanged

#### Scenario: Signing is unavailable
Given the repository signer cannot be unlocked or reached
When Git attempts to push a reference update
Then the push fails before accepted references change
And no unsigned transition is created

### Requirement: The remote helper has no network authority

`git-remote-styrene` communicates only with the authenticated local `styrened` IPC
endpoint. It cannot open RNS links, configure tunnels, contact discovered peers, or bypass
daemon authorization and policy.

#### Scenario: Fetch requires synchronization
Given the requested repository is not available in local verified storage
When Git invokes the remote helper
Then the helper returns a typed local-not-available result or requests a daemon sync job
And only `styrened` may choose a peer and carrier for that job

#### Scenario: Daemon is unavailable
Given no authenticated local daemon IPC endpoint is available
When Git invokes the remote helper
Then the operation fails without falling back to direct networking
And repository state remains unchanged

#### Scenario: Unauthorized local caller
Given a local IPC caller lacks the capability to modify a repository namespace
When the caller requests a push
Then `styrened` denies the request before invoking repository mutation

### Requirement: Helper behavior is testable without a production bypass

The command loop, typed IPC client, and Git-plumbing adapter have deterministic test seams.
The final process-level acceptance test uses ordinary Git, the installed helper name, and
the shipped authenticated daemon adapter. Production binaries contain no environment flag
or hidden command that bypasses daemon authorization.

#### Scenario: Deterministic command-loop fixture
Given an in-memory authenticated transport and fake Git-plumbing adapter
When tests exercise capability, list, fetch, push, force, deletion, limit, and malformed-input cases
Then emitted protocol lines and typed IPC requests are deterministic

#### Scenario: Real Git black-box workflow
Given the shipped helper and authenticated daemon Git service are installed on `PATH`
When a test initializes, pushes, clones, fetches, and checks out using only `styrene://`
Then Git completes through the production helper and daemon interfaces
And no test-only transport or authorization bypass is enabled
