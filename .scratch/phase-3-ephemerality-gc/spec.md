# Spec: Ephemerality GC — expired non-public posts actually leave

Status: ready-for-agent

Phase 3 of the post-encryption rollout (`docs/2026-07-05-post-encryption-spec.md`).
Turns expiry into real teardown of recoverable content for encrypted
(Friends/Circles) posts, on the author's device and on every recipient's.

## Problem Statement

When I let a post go — give it a lifetime so it expires, or delete it — I
expect it to actually disappear. Right now it only vanishes from view: the
app stops *showing* an expired post, but its encrypted contents and its
photos/videos quietly persist on my device and on every friend's device
that synced them, and those devices keep serving the media to anyone who
asks. So "ephemeral" isn't really ephemeral — the sensitive part (the
readable text and the media files) outlives the moment I said it should be
gone. Because the whole privacy model deliberately relies on *retention*
(deleting expired content) rather than forward secrecy, this gap means the
central promise of the feature isn't currently kept.

## Solution

Expiry becomes real deletion of everything decryptable. When a non-public
post expires — or when I delete it — its media files are torn down on every
device that holds them, mine and my friends', and the locally-cached
decrypted copy of its text is purged. Devices that were offline at the
moment of expiry clean up as soon as they next start, so being offline only
delays teardown, never prevents it. Posts I explicitly kept are unaffected —
a keep is its own promise to retain, and it survives the original's expiry
until I release it.

The only thing that may linger is the **header** of the encrypted operation —
metadata (author, sequence number, timestamp, payload size, payload hash,
backlink, signature), never the ciphertext and never anything decryptable. The
encrypted *payload* itself can be deleted: p2panda models a body as optional
(`Operation.body: Option<Body>`) and the store exposes `delete_operation_payload`,
which drops an operation's body while keeping its header, so the backlink chain
and sync stay intact (validation references the header's `payload_hash`, not the
body). Deleting the payload is therefore safe from any position in the log —
what would break the chain is deleting a *header* from the middle, which we
never need to do. So no image, no video, no readable text, and no ciphertext
survives; the residue is header metadata, called out explicitly rather than
hidden.

(Payload deletion is now implemented: teardown calls `delete_operation_payload`
on an expired/deleted post's content-bearing wrapper op(s), so the ciphertext is
erased alongside the decrypted-plaintext purge + media teardown. The first Phase
3 cut shipped without it; it landed as the strict strengthening described here.)

## User Stories

### Letting a post go (author)

1. As a poster, I want an expired Friends post's photo to be removed from my
   own device, so that letting a post go actually reclaims the space and the
   content.
2. As a poster, I want the locally-decrypted text of my expired post purged
   from my device, so that someone who later gets access to my disk can't
   recover what it said.
3. As a poster, I want deletion (not just expiry) to trigger the same
   teardown, so that "delete" is as thorough as "let it expire."
4. As a poster, I want an expired post with several attachments to have all
   of its attachments torn down, so that partial teardown never leaves one
   image behind.
5. As a poster of a permanent (never-expiring) post, I want it and its media
   left completely untouched, so that GC only ever touches content I chose
   to make ephemeral.

### On my friends' devices (recipients)

6. As a poster, I want an expired Circles post's video to be removed from the
   devices of the friends and friends-of-friends who received it, so that my
   audience can no longer open it after it expired.
7. As a friend who received and viewed a post, I want its media purged from
   my device once it expires, so that I'm not silently holding my friend's
   expired content.
8. As a friend, I want to stop serving an expired post's media to other peers
   once it has expired, so that I'm not a source that keeps expired content
   alive on the network.

### Offline and restore

9. As a poster whose device was offline when a post expired, I want the
   teardown to happen the next time my device starts, so that expiry is
   eventually enforced regardless of connectivity.
10. As a friend whose device was offline at expiry, I want the same catch-up
    teardown on my next start, so that offline recipients converge to the
    same "gone" state.
11. As a user restoring from backup, I want expired posts to be absent and
    their media not re-imported, so that restore never resurrects content
    that had already expired.

### Keeps

12. As a user who kept a post, I want that kept copy — text and media — to
    survive the original post's expiry, so that keeping still means keeping.
13. As a user who kept a post, I want its media to remain available even
    after the author's copy and every other recipient's copy have been torn
    down, so that my keep is genuinely independent.
14. As a user, I want releasing a keep after the original has expired to then
    reclaim that media, so that the keep is the last thing holding it and
    letting go finishes the teardown.

### Honesty and unchanged behaviour

15. As a privacy-conscious user, I want a clear, honest statement of exactly
    what "gone" guarantees: media and readable text are deleted, the encrypted
    payload can be deleted too, and at most a small operation *header*
    (metadata, no content) may remain — so I understand what does and doesn't
    survive.
16. As a user with expired public posts, I want the app to keep behaving as
    it does today (expired public posts simply stop showing), so that this
    change doesn't unexpectedly alter public-post behavior.

## Implementation Decisions

- **Scope: replicated non-public posts (Friends and Circles).** Private
  (local-only) posts and kept copies already tear down on expiry/lease-lapse;
  the gap this closes is replicated encrypted posts, which today never tear
  down their media on the author's side and are never drained on recipients'
  side. Public posts stay as they are (read-time filtering only).

- **Expiry teardown, author side.** The expiry-drain path (today it only
  drains local private posts and prunes lapsed keeps) is extended to walk the
  author's own *replicated* posts that have expired and tear down their
  attachments the same way an explicit delete already does: remove the post's
  attachment pins so the blobs lose their GC root, and prune the
  materialized plaintext cache files. Reduction and read-time filtering are
  unchanged — teardown is a side effect layered on top, not a change to how
  state reduces.

- **Expiry teardown, recipient side.** A recipient learns a post's expiry
  time from the decrypted payload it already holds. On the drain pass, a
  recipient tears down the attachments of any decryptable post that has
  expired: prune the plaintext cache file and drop the recipient's hold on
  the synced ciphertext blob so it becomes GC-eligible on that device too.
  This is the new behavior that lets expired content leave *the network*,
  not just the author.

- **Decrypted-plaintext purge.** The per-operation decrypted-payload cache
  (the table that lets the spaces service substitute a decrypted inner
  operation into reduction) has its row for an expired post deleted during
  teardown, so the readable form does not survive on disk even though the
  encrypted operation does.

- **Keeps are pin-counted by construction and must stay that way.**
  Attachments are pinned under distinct namespaces — one for a post's own
  feed presence, a separate one per keep. Teardown removes only the feed
  namespace's pins; a kept post's separate pins keep its blobs alive until
  the keep is released or its own lease lapses. This preserves the existing
  guarantee that a keep is independent of the original, and it is the
  mechanism that makes "expire the post but not the kept copy" correct: a
  blob referenced by both is reclaimed only when the *last* referencing pin
  is gone.

- **Offline convergence via drain-on-startup.** The startup recovery path
  already drains expired private posts and enforces keep leases; it is
  extended to run the same replicated-post teardown, so a device that was
  offline at expiry converges on next start. Teardown is idempotent —
  re-running it on an already-torn-down post is a no-op.

- **Delete reuses the same teardown.** Explicit deletion already removes a
  post's feed pins; it additionally purges the decrypted-plaintext cache row,
  so delete and expiry converge on the same end state.

- **Phase 3 now deletes the ciphertext payload of expired/deleted posts (shipped
  after the first cut).** Teardown's shared helper `erase_post_content` drops the
  payload (body) of a post's content-bearing (`PostPublished`/`PostEdited`)
  wrapper op(s) via the store's `delete_operation_payload`, which keeps the header
  — so the backlink chain and sync stay intact (validation is against the header's
  `payload_hash`, and a body-less operation is a valid, modeled state that
  `operations_for_profile` now skips). Tombstone/lifetime wrapper bodies are kept
  so reduction still reflects delete/promote. This is distinct from removing the
  whole operation (header included) from the middle of a log, which *would* break
  the next operation's backlink; that is the thing we never do, and it is
  unnecessary, because deleting the payload already erases the ciphertext. No
  jyn-side schema change was required. Note: `p2panda-core`'s `PruneFlag`
  (network-wide *prefix* GC via `p2panda-stream`) is a separate, coarser tool — it
  drops everything before a point, so it does not map to deleting one expired post
  from the middle; per-operation `delete_operation_payload` is the right primitive
  there.

- **We do not test, drive, or wait on the underlying store's asynchronous
  garbage collector.** jyn's responsibility ends at removing the GC root (the
  pin) and the cache file; reclaiming the bytes is the store's job on its own
  schedule. The observable contract jyn owns is "the pin and the cache file
  are gone, and the decrypted-plaintext row is gone," all synchronous with
  the drain.

## Testing Decisions

- **A good test asserts external, deterministic behavior — never a timer.**
  Tests must not sleep waiting for wall-clock expiry, and must not wait for
  the store's async GC to reclaim bytes. Expiry is made deterministic by
  giving a post an `expires_at` already in the past (set its lifetime to a
  past instant) and then triggering the drain explicitly; teardown is
  asserted on the state jyn synchronously controls (pins removed, cache file
  pruned, decrypted-plaintext row absent). If a test ever needs to observe
  the *physical* absence of bytes rather than the removal of their GC root,
  it does so through a mock/seam over the blob store, not by waiting for real
  GC.

- **Primary seam — the network bridge command/event interface.** The same
  seam the existing multi-node integration tests already drive over a relay.
  Covered behaviors: author-side media teardown after expiry; recipient-side
  media teardown after expiry (two- and three-node, mirroring the
  friends-of-friends setup); delete-triggers-teardown; keep-survives-expiry
  and release-after-expiry-reclaims; multi-attachment posts; permanent posts
  left untouched; offline-then-restart convergence (reusing the existing
  drop-and-respawn restart pattern).

- **Secondary seam — the operation-domain unit level.** The
  decrypted-plaintext purge is a security property not observable through the
  bridge (the feed already hides expired posts by read-time filtering, which
  would pass without the cache actually being purged). It is asserted
  directly at the domain seam: after teardown, the decrypted inner operation
  for an expired post is no longer retrievable.

- **Prior art.** Integration behaviors follow the existing relay-driven
  multi-node tests (the friendship, circles, and backup-media suites) for
  spawning onboarded nodes, forming friendships, publishing, syncing, and
  fetching media. Domain-level assertions follow the existing reduction unit
  tests. Restart/backup behaviors follow the existing service-restart and
  backup-round-trip tests.

## Out of Scope

- **Deleting the ciphertext payload of expired operations — DONE (shipped after
  the first cut).** Both changes landed: (a) `erase_post_content` (the shared
  teardown helper called by both the author- and recipient-side paths) calls
  `delete_operation_payload` on an expired/deleted post's content-bearing
  (`PostPublished`/`PostEdited`) `Spaces` wrapper op(s), scoped by
  `content_post_id()` so tombstone/lifetime bodies are left intact for reduction;
  (b) `operations_for_profile` now skips a body-less operation instead of erroring
  (the old `body.context("payload is missing")?` became a `let Some(body) = … else
  { continue }`). Both network-wide concerns were resolved, so no re-acceptance
  gate was needed: sync will *not* re-materialize a deleted body — `ingest_operation`
  early-returns on `has_operation`, which matches by row regardless of body — and a
  drained peer serving a body-less op to a fresh peer is protocol-native (LogSync
  models `Body` as optional and ingest validates backlinks, not `payload_hash`), so
  the fresh peer receives header-only metadata. Convergence rides on the per-device
  drain, exactly as recipient teardown does.
- **Erasing the operation *header* / metadata.** Removing a header from the
  middle of a log breaks the next operation's backlink and sync, and is
  unnecessary for confidentiality — the header carries no readable content and
  no ciphertext, only metadata (author, seq, timestamp, size, `payload_hash`,
  backlink, signature). This residue is out of scope and, unlike the payload, is
  a genuine protocol-level constraint.
- **Reshare and reshare-by-reference dedup.** There is no reshare feature in
  the product today, so the by-reference dedup optimization has nothing to
  attach to. Descoped; the design is recorded under Further Notes for
  whenever reshare is built.
- **Public-post media teardown on expiry.** Public posts and their media are
  plaintext by design and carry no confidentiality promise; their expiry
  stays read-time filtering only. Reclaiming expired public media is a
  storage optimization that can be considered separately.
- **Multi-device.** Unchanged from Phase 1 — still deferred pending upstream.
- **The honest ephemerality statement in the UI** (user story 15). Descoped
  from Phase 3 implementation and recorded as a durable follow-up in
  `docs/2026-07-05-post-encryption-spec.md` (Deferred follow-ups). It is a
  copy/UI change with no dependency on the GC engine; do not re-create it as
  a Phase 3 ticket.

## Further Notes

- **Reshare-by-reference (for a future reshare feature).** When reshare
  exists, a reshare should reference the *original* ciphertext blob (same
  content address) and re-wrap the original per-blob key to the resharer's
  audience, so the bytes are stored once network-wide rather than
  re-encrypted. This composes with pin-counting: a blob referenced by
  several posts (original + reshares) is reclaimed only when the last
  referencing post is torn down.

- **Ciphertext-payload deletion is available today (correction).** An earlier
  draft of this spec treated log pruning as a missing upstream capability. That
  was wrong: p2panda already exposes `delete_operation_payload` (drop a body,
  keep the header — chain-safe) and `PruneFlag` (network-wide prefix GC via
  `p2panda-stream`). So there was no upstream ask blocking ciphertext erasure;
  the first Phase 3 cut left ciphertext in place purely for scoping, and payload
  deletion has since shipped (see Out of Scope). The accurate user-facing
  statement is: expired media and readable text are gone, and the encrypted
  payload is erased too — what may remain is operation-header metadata, not
  content. Confirmed against `p2panda-store`
  (`operations/traits.rs`, `operations/sqlite.rs`) and `p2panda-core`
  (`operation.rs`, `prune.rs`) on 2026-07-13.

- **Threat-model honesty.** `keep_post` intentionally lets any recipient
  retain a permanent private copy, so cryptographic ephemerality against
  one's own friends was never the goal. This phase makes the *default*
  path (no keep) actually delete recoverable content; it does not, and
  cannot, revoke copies a recipient deliberately kept.
