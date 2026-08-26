# Bootstrap self-certifying Styrene repositories

## Intent

Establish the network-independent core of `styrene-git`: repositories whose identity,
governance, peer namespaces, and published references can be verified from repository
data alone, regardless of which peer or Styrene carrier supplied that data. Integrate
ordinary Git commands through a local `styrened` IPC boundary so the daemon remains the
sole owner of discovery, links, tunnels, and network policy.

## Scope

Included:

- Stable repository identifiers derived from a canonical initial identity document.
- Styrene Identity signer bindings, delegate policies, and threshold-authorized identity updates.
- Independent verification of the released Styrene Identity repository-signing corpus.
- Bare Git storage with an independently authenticated namespace for each publisher.
- Replay-resistant signed reference transitions and deterministic canonical default-branch selection.
- A `styrene://` Git remote helper that performs local operations through typed daemon IPC.
- A transport-neutral replication contract that `styrened` can carry over RNS Resources,
  direct streams, negotiated tunnels, or future carriers.

Excluded:

- Concrete public-network discovery, WireGuard, Yggdrasil, I2P, and routing implementations.
- Issues, patches, reviews, notifications, repository search, and user interfaces.
- Private-repository content encryption and recipient revocation.
- GitHub, GitLab, Forgejo, or Radicle migration and compatibility tooling.
- Large-file extension protocols and CI execution.

## Success criteria

- Two independent implementations can derive the same repository ID from the same
  canonical identity fixture and reject a semantically or bytewise invalid encoding.
- A repository clone obtained from an untrusted source can verify its identity history,
  publisher namespaces, signed reference transitions, and canonical default branch
  without consulting that source as an authority.
- Replayed, reordered, cross-repository, unauthorized, and incomplete updates are rejected
  without changing accepted repository state.
- Repository authority uses the released Styrene Identity repository-signing profile rather
  than a Git-local identity identifier, key-purpose, or binding definition.
- Git can push to and fetch from local Styrene storage through `styrene://` while offline.
- Network synchronization requests cross a typed local IPC boundary, and only `styrened`
  selects or operates the underlying carrier.
- The delta specifications, design, and implementation tasks validate as one OpenSpec change.
