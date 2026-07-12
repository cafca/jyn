# Members-only Groups reuse the p2panda-spaces Manager protocol per GroupId

**Status:** accepted

The members-only key-delivery / re-key mechanism is **not** a new protocol to
design. `JynSpaces` already implements the whole flow against `p2panda-spaces`
for Friends/Circles, and Groups is built on the same crate
([ADR-0004](0004-groups-as-standalone-subsystem.md)). The Groups subsystem
instantiates a `p2panda-spaces` space per GroupId and drives it with the same
`Manager` API and the same log/message conventions, over the Group's own
replication topic ([ADR-0007](0007-group-replication-topic.md)).

## The reused mechanisms (as in `core/src/spaces/mod.rs`)

- **Key-bundle publication** — each member publishes/refreshes a key bundle
  (`manager.key_bundle_message()` / `key_bundle_expired()`), surfaced as
  `Event::KeyBundle` on ingest.
- **Key delivery on admit** — adding a member emits a membership control
  message carrying a **welcome payload** that hands the new member the group
  secret, so they can decrypt. This works regardless of friendship, because the
  control message flows on the Group topic.
- **Lazy re-key on removal** — removing/leaving re-keys the space right before
  the next Group post (`repair_spaces` / `remove_stale`), bounding re-key to
  publish frequency, not churn.
- **Trial-decrypt** — recipients run `manager.process(...)` in a
  `process_message` equivalent; `Event::Application` yields the decrypted inner
  operation, and which space a post targets is learned from posts that decrypt.
- **Own payloads** stored decrypted at authoring time; never re-processed.

## Why this closes the open question

The only differences from `JynSpaces` — N named spaces addressed by GroupId
instead of two per-profile spaces, and members who are not the owner's friends —
are already within the `Manager` model (being a `Write` member of another
profile's space already happens for friends; the crate does not restrict space
count). No bespoke byte-level protocol is invented. Implementation can proceed
without a design spike.
