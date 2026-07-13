# 03 — Members-only (encryption) end to end

**What to build:** The encrypted Content mode. A Group can be created
members-only, in which case its posts, comments, and media replicate as
ciphertext readable only by members. Layers `p2panda-spaces`-per-GroupId onto
the container from ticket 01, reusing the existing `JynSpaces` protocol flow —
no new wire protocol. Proven at the `AsyncBridge` seam (multi-node), parallel to
`core/tests/circles.rs`. Respect ADR-0002, ADR-0003, ADR-0007, and ADR-0015.

**Blocked by:** 01, 02.

**Status:** ready-for-human

- [x] A Group can be created with Content mode `members_only` (fixed at creation); its auth group (from ticket 01) is unchanged, and an encrypted `p2panda-spaces` space is instantiated per GroupId.
- [x] Group posts, comments, and media in a members-only Group are encrypted to the Group's space; a non-member (and any passive peer) sees only ciphertext and cannot read content or enumerate the roster.
- [x] Admitting a Member delivers the group secret via the welcome-payload mechanism on the add-member control message, so the new Member can decrypt; this works whether or not the Member is a friend of the Owner, and in both Open and Request-to-join modes (including the offline-Owner pending case).
- [x] Recipients trial-decrypt on ingest; a member's own payloads are stored decrypted at authoring time.
- [x] Removing or leaving a members-only Group triggers a lazy re-key right before the next Group post; the removed Member cannot read posts made after removal.
- [x] Content the removed Member already received is not clawed back (honest no-clawback contract — see ticket 04 for the phase-3 follow-up).
- [x] Roster visibility for a members-only Group is members-only; hearts on members-only posts never produce an outward discovery card.
- [x] Key-delivery / re-key reuses the `p2panda-spaces` `Manager` flow as `JynSpaces` does (key-bundle publication, welcome-on-add, `repair`/`remove-stale` lazy re-key, trial-decrypt) — no bespoke protocol.
- [x] Integration tests at the `AsyncBridge` seam cover: members-only posts reach members but not non-members; a joiner gains read access after admission; removal re-keys the removed member out of the next post; ciphertext-only for non-members.

## Comments

Implemented 2026-07-12. `create_group(members_only)` instantiates a
`p2panda-spaces` space with the GroupId as the space id; sealing, welcome
delivery, trial-decrypt, and lazy re-key reuse the Manager flow verbatim
(ADR-0015). Key bundles travel over profile topics (`sync_group_peer_profiles`:
Owner syncs joiners, members sync the Owner). Space membership mirrors the
reduced roster — adds eager, removals only in the pre-publish re-key.
Proven by `core/tests/groups_members_only.rs`.

Notes:
- `space.add` forges its auth message before building the welcome, so the
  service checks the key registry for the joiner's bundle first — otherwise a
  missing bundle leaves a dangling half-add on the wire.
- The pinned p2panda auth resolver recurses deeply once several groups/spaces
  share the auth graph; the bridge runtime now runs 16 MiB worker stacks
  (default 2 MiB overflowed).
- Wire-level caveat (per the spec discussion): membership *control* messages
  are plaintext on the group topic — as they are for the friends space today —
  so roster confidentiality is enforced at the bridge/API surface, which is
  what the seam test asserts.
