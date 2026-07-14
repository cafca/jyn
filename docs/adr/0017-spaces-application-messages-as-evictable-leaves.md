# p2panda-spaces application messages must be evictable leaves for encrypted content to be deletable

**Status:** proposed

[ADR-0016](0016-logs-are-expiry-keyed-co-deletion-units.md) makes each post a
co-deletion unit so that expiry and deletion become real storage events. For
plaintext posts this is complete: dropping the bucket log removes the
operations. For **encrypted** (Friends / Circles / members-only) posts it is
not, and the gap is upstream, in `p2panda-spaces`.

An encrypted post is an `SpacesArgs::Application` message carried in a
`DomainOperation::Spaces` wrapper on a p2panda log (`core/src/spaces/forge.rs`);
its ciphertext lives in `operations_v1` and deletes with the log. But the
`p2panda-spaces` encryption orderer records **every** message — application
messages included — as a permanent node in the space's derived state:
`space.rs` `publish()` and `handle_application_message()` both call
`y.encryption_y.orderer.add_dependency(message.hash(), &deps)`, which
(`encryption/orderer.rs`) inserts the hash into the graph and recomputes
`heads`, so the *next* message takes the application message as a causal
dependency. That derived state is persisted (`set_space_state_tx`, `spaces_v1`).

Two consequences: (1) deleting an application message's ciphertext still leaves
its **hash and DAG position** as residual metadata, and (2) later messages
*depend* on it, so a peer that GC'd it cannot order what came after.

## Decision (proposed, upstream)

**Application messages must be leaves in the space's causal graph, and must not
be retained as permanent orderer nodes.** Concretely, in `p2panda-spaces`:

- An application message depends only on the **current control/key-epoch heads**
  — enough to select the group secret (`group_secret_id`) a recipient needs to
  trial-decrypt — carried in its `space_dependencies` as today.
- It **never becomes a dependency head** itself: no later control or application
  message points at it. Publishing or receiving an application message must not
  advance the frontier that subsequent messages build on.
- It is **not persisted** as a node in the orderer graph. Once decrypted (or
  once deleted), it can be dropped from the space's derived state with no
  dangling reference.

This is sound in the data-encryption scheme (`p2panda-encryption` `data_scheme`):
application messages **do not mutate group state** — only control messages
(add / remove / update) do — so they do not belong in the causal graph that
orders state changes. The graph should track control messages; application
messages hang off it as removable leaves.

**Locus:** the fork `cafca/p2panda`, `p2panda-spaces`
(`src/space.rs` `publish` / `handle_application_message`,
`src/encryption/orderer.rs` `add_dependency` / `next_application_message`), and
the `data_scheme` send/receive paths as needed.

## Consequences

- **Unblocks true deletion of encrypted content.** With application messages as
  evictable leaves, dropping an encrypted post's bucket log removes the
  ciphertext *and* leaves no orderer residue — the "truly deletable, no
  metadata" property [ADR-0016](0016-logs-are-expiry-keyed-co-deletion-units.md)
  wants for members-only contexts.
- **Until it lands, ADR-0016 ships the accepted interim caveat** — encrypted-
  post deletion removes content but leaves the message hash and its graph
  position. No content and no expiry leak; only that an op existed.
- **Ordering for decryption must be preserved.** The change must keep enough
  ordering that a recipient can still pick the correct group secret for an
  application message; its `space_dependencies` naming the key epoch is
  sufficient, so leaf status does not weaken decryptability.
- **Coordinate with the reuse of the Manager.** jyn drives the same
  `p2panda-spaces` `Manager` for Friends / Circles and for Groups
  ([ADR-0015](0015-members-only-reuses-p2panda-spaces-protocol.md)); the change
  is protocol-level and benefits both. Pin the fork revision when it lands and
  flip both this ADR and the ADR-0016 residual note.
- **Risk.** `p2panda-spaces` / `p2panda-auth` / `p2panda-encryption` are young
  and moving (per the encryption spec's open risks); this is an upstream
  protocol change on the critical path for deletion, so budget for API churn and
  restore-testing of the derived state across the change.
