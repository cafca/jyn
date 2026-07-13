# Logs are co-deletion units keyed by expiry, not semantic streams

**Status:** accepted

Today a profile has six fixed logs, each addressed by a *semantic*
`DomainLogId { profile_id, kind }`, and the sync topic is derived from the same
id (`profile_sync_topic`, `core/src/domain.rs`). Every post an author writes
lands in the one shared `Posts` log — or, for non-public posts, the one shared
`Spaces` log carrying `DomainOperation::Spaces` wrappers (`core/src/spaces/forge.rs`).
Nothing is ever deleted: a tombstone only hides a post in the *reduced* state,
while the raw operations replicate forever. Expiry and explicit deletion are
read-time filters, not storage events — real removal is deferred to
"Phase 3 — Ephemerality GC of expired ciphertext/blobs"
(`docs/2026-07-05-post-encryption-spec.md`).

This is a misuse of the `log_id` primitive. A p2panda log is the finest thing
the store can delete as a whole (`LogStore::prune_entries` removes a contiguous
prefix of one `(author, log_id)` log), yet today's logs mix unrelated posts
with unrelated lifetimes, so no individual post is ever at a prunable boundary.
This ADR reframes the primitive: **a log is the set of operations we intend to
delete together**, and pins how content is *placed* into such logs.

**Scope.** This ADR decides **placement and addressing** only — which log a
post, edit, re-home, or reaction is written to, and how logs map to topics.
The **garbage collection** that acts on this structure — dropping an expired
bucket, un-pinning its blobs, reaping or rolling forward reactions — is **left
to the GC workstream** (encryption Phase 3). Placement pins content *in place*
(a bucket's operations and their blob pins are retained until GC); GC owns
removal. The two are separable precisely because placement makes each bucket a
clean deletion unit.

This is a breaking on-disk change, shipped as a flag-day wipe with a
`DATA_SCHEMA_VERSION` bump (`core/src/data_schema.rs`), consistent with the
encryption spec's migration stance.

## Decisions

**A log is a co-deletion set, addressed by an opaque id.** The `log_id` stops
encoding what a log *is* (`profile` / `posts` / a specific profile). It becomes
a meaningless handle whose only job is to name a bundle of operations that live
and die together. What the operations are *about* — which profile or group is
their audience — moves out of the id and into a header field (see the topic
decision below). Reduction is unaffected in shape:
`TopicStore::resolve(topic)` already returns *all* associated
`(author, log_id)` pairs regardless of how the ids are structured, so "six
known kinds" becomes "whatever logs are currently associated," and a GC'd log
simply drops out of `resolve`.

**Ids are reserved-then-monotonic, never reused.** Allocation is per-author and
*global* — in p2panda a `log_id` is scoped only to the author
(`operations_v1` is keyed `(verifying_key, log_id, seq_num)`; the topic is an
association tag on top), so one author's profile logs and their logs inside a
members-only group draw from a single id space and must not collide.

- **`0`–`999` — reserved fixed ids** for the author's own singleton logs
  (profile, contacts, incoming requests, …), known at compile time so no lookup
  is needed to address them.
- **`≥ 1000` — a monotonic counter** for every dynamically created log: expiry
  buckets and each group's control/bucket logs alike. Allocate the next
  never-used integer; **never reuse a retired id.** Reuse is unsafe across
  peers: a fresh log at a recycled id restarts at `seq_num 0`, and any peer that
  still holds the old log rejects it — `validate_prunable_backlink`
  (`p2panda-core`) runs `validate_backlink` against the stale local head and
  fails. Deleting the old log locally cannot prove it is gone from every peer,
  so a recycled id resurrects as a conflict on laggards. u64 ids are free;
  monotonic growth costs nothing.

The `bucket → log_id` mapping is **local authoring state**: only the author's
GC needs to know "log 1042 is the 7d bucket closing 2026-07-19." Readers never
need it — they fold every associated log and read `expires_at` straight from
each payload.

**Posts are placed by expiry, coarsened to the lifetime chip.** Lifetimes are a
fixed ladder of chips (e.g. `1h / 1d / 7d / 30d / permanent`). Each chip is a
tier with its own bucket series, and **granularity equals the chip's own
duration**: a 7d post uses 7d-wide buckets, a 1h post uses 1h-wide buckets. A
post's bucket is `(chip, floor(expires_at / granularity))`; posts of the same
chip whose expiries fall in the same window share one log. This bounds the
window a bucket covers to one granularity, so when GC later drops it, over-
retention is **at most one granularity** past a post's own expiry (a 7d post
≤ 7d extra), i.e. ≤ 100% of its lifetime — the accepted GC granularity.
Permanent posts never expire, so they are placed by **post month** instead (a
coarse time index; a permanent post is deleted individually, not by draining
its month).

**Lifetime change re-homes the post as a self-contained snapshot, and
tombstones the old copy.** Changing a post's lifetime changes its bucket. The
author re-publishes a **complete current snapshot** of the post (body + current
media refs + state, collapsing prior edits) into the correct new bucket log,
and writes a **tombstone** into the old bucket so the stale copy is removed
(by GC) rather than left to shadow. Reduction dedupes by `post_id`, newest
`ordering_timestamp` wins, and `next_ordering_timestamp` already guarantees the
snapshot sorts after everything it supersedes (`core/src/domain.rs`). Because
the snapshot is self-contained, a post survives the eventual GC of every
earlier bucket it passed through.

**Reactions and comments are placed by the post's expiry; their lifetime is the
post's, enforced by GC.** This supersedes the earlier "comment carries its own
write-time deadline and never moves." A reaction is single-author (it lives in
the reactor's log, on the reactor's topic), so it cannot co-delete atomically
with a foreign-authored post. This ADR pins only the **placement**: a reaction
is written into the bucket of the post's `expires_at` *at write time*, so in the
common case (post expires on schedule) it drains with the same bucket. Its
**lifetime enforcement is the GC task's responsibility** — keep while the post
is live in the post-author's synced state, reap when the post is tombstoned or
drained, roll forward if the post is promoted, and fall back to the post's
last-known expiry when the author is no longer synced (no infinite orphans).
The requirement handed to GC is simply: *a reaction lives exactly as long as the
post it is on.*

**The sync topic derives from a header context field; members-only contexts may
blind it.** The topic moves off the `log_id` and onto a header field naming the
audience (a profile id, or a group handle). This is a free decoupling —
`ingest_operation(store, op, log_id, topic, prune)` already takes `log_id` and
`topic` as separate arguments (`p2panda-stream`). For **members-only** contexts
the topic may be *blinded*: derive it as `H(shared_group_handle)` from a random
handle known only to members, so a passive observer sees gossip on an opaque
hash it cannot tie to any named group. Constraints:

- **Public content cannot be blinded** — a new follower must derive the topic
  from the public profile id, so the technique is members-only.
- **Derive from a stable handle, not the rotating epoch secret**, or the sync
  rendezvous would move on every re-key and split history across epochs; rotate
  keys underneath a fixed handle. Aligns with the per-context replication axis
  of [ADR-0007](0007-group-replication-topic.md).
- **The header carries the opaque handle, not the plaintext audience id**, or
  the field itself de-blinds it.
- Blinding hides *which* context, not the *existence* of a shared topic or
  network-level membership clustering (peers still announce interest in the
  hash).

## Consequences

- **Non-semantic ids close the expiry leak.** Because a bucket's real key lives
  only in the author's local map, the cleartext `log_id` is an arbitrary
  integer that reveals nothing about a post's expiry — even for encrypted posts
  on a blinded group topic. Blinded topic hides the group, opaque log id hides
  the expiry, encrypted payload hides the content; the three line up.
- **Placement hands GC clean deletion units.** Dropping a whole bucket once its
  window passes, reaping/rolling reactions, and reference-counting blob pins
  across a re-home (the snapshot reuses the same `blob_hash`, so un-pinning must
  not drop media the live copy still references, `[[blob-lifecycle]]`) are all
  **GC-owned** and specified by that workstream, not here.
- **Sync cost scales with live-log count, not post count today.** Moving from
  ~6 logs/profile to O(live buckets)/profile grows per-log height
  reconciliation proportionally, and GC'd logs must stop being announced.
  Coarse chips keep the live-bucket count small; this is the main thing traded
  for granular deletion.
- **Logs are single-author, so co-deletion is single-author.** A post and its
  own edits/lifetime/tombstone share a log; a *foreign* reaction cannot. The
  cross-author binding ("a reaction lives as long as its post") is delivered by
  GC's reactive enforcement, not by log structure.
- **Encrypted-post deletion carries an accepted residual for now.** For
  encrypted posts, dropping a bucket log removes the ciphertext operation
  cleanly, but the `p2panda-spaces` encryption orderer persists every
  application message's hash in the space's derived state and chains later
  messages onto it (`space.rs` `add_dependency`). Until that is fixed upstream —
  tracked as the proposed
  [ADR-0017](0017-spaces-application-messages-as-evictable-leaves.md) — deleting
  an encrypted post removes its content but leaves the message hash and its
  position in the orderer graph. This residual is **accepted** as a known,
  bounded limitation of the initial rollout; it leaks no content and no expiry,
  only that *an* op existed. Reuses the `JynSpaces` mechanics per
  [ADR-0015](0015-members-only-reuses-p2panda-spaces-protocol.md).
- **Flag-day wipe.** Operation layout, log addressing, and topic derivation all
  change incompatibly; ship behind a `DATA_SCHEMA_VERSION` bump that wipes
  stores while keeping the identity keypair, as in the encryption flag day
  (`core/src/data_schema.rs`).
