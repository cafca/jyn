# 04 — Pre-merge check + final verify

**What to build:** The close-out gate. Resolve the encryption-phase-3 dependency
for the removal-clawback contract, then verify the whole feature end to end
before merging. Respect the spec's "Pre-merge checks" section and ADR-0003.

**Blocked by:** 03.

**Status:** ready-for-human

- [x] Determine whether encryption **phase 3 (garbage collection of expired ciphertext + blobs)** has landed on `main`.
- [x] If phase 3 is on `main`: bring the GC-based tightening of removed-member retention (spec story 28) into scope — revise the no-clawback contract and removal semantics so removal reduces retained content via GC-driven expiry, and implement it with tests.
- [x] If phase 3 is not on `main`: leave story 28 as the honest no-clawback contract and record the GC-based tightening as a follow-up (not part of this merge), with a note pointing at the phase-3 dependency.
- [x] The full `AsyncBridge` integration suite (public + members-only Groups, join/governance, discovery, hearts) passes over a real relay; the Flutter provider-level tests pass.
- [x] The alignment rules in ADR-0014 hold in the shipped code (role-as-set, append-only membership, extensible governance op set, nothing anchored to the creator, Group as a first-class actor, composable identity/topic).

## Comments

Closed out 2026-07-12.

**Phase-3 check:** encryption phase 3 (ephemerality GC) has NOT landed on
`main` — commit b9eb2c2 there adds only the design spec and tickets
(`.scratch/phase-3-ephemerality-gc/`). Story 28 therefore ships as the honest
no-clawback contract. **Follow-up (not part of this merge):** once phase 3
lands, extend its author-/recipient-side teardown to removed-member retention
in members-only Groups — a removed member's node should tear down expired
sealed group content the same way expired circle content is torn down; the
group's decrypted-plaintext cache rows (`jyn_spaces_decrypted` keyed by the
group wrapper ops) are the additional surface to cover.

**Full verify:** `cargo test -p jyn` green (82 unit tests; integration over a
real relay: friendship x3, circles, backup_media, groups_public x2,
groups_discovery, groups_members_only). `cargo clippy --all-targets -- -D
warnings` and `cargo fmt --check` clean. `flutter analyze` clean; provider
tests pass.

**ADR-0014 alignment rules in the shipped code:**
- *Role-as-set*: `GroupMemberEntry.roles: Vec<GroupRole>`; every check routes
  through `permitted_actions` (union over held roles). No owner boolean
  exists anywhere in the subsystem.
- *Append-only membership*: membership is only ever ops
  (`GroupGoverned`/`GroupJoinRequested`/`GroupLeft`) reduced in
  `groups::reduce`; `membership_history` preserves the who-could-read
  timeline across leaves/removals.
- *Extensible op set*: `GroupGovernanceAction` is a tagged serde enum, and
  reduction skips undecodable operations instead of failing, so future
  moderation/role ops add without a schema break.
- *Post-presence separable from author-delete*: group posts live on group
  logs keyed by post id in the group reduction; a future moderation-hide op
  can act on that presence independent of `PostDeleted`.
- *Nothing anchored to the creator*: only the GroupId derives from genesis;
  metadata/membership anchor to the current `Manage` holder, seeding to the
  member set — proven by the transfer-then-leave integration test.
- *First-class actor / composable*: the group has its own topic, its own log
  namespace, and its own auth entity; members are `VerifyingKey` actors via
  `GroupMember`, which upstream already generalizes to group-as-member.
- *Litmus test*: an auto-derived, blinded, always-encrypted per-profile group
  is expressible — a members-only group whose governance ops are driven by a
  derivation function instead of UI commands; nothing in the model requires a
  human-set name, advertisement, or open joining.

## Multi-agent code review (pre-commit)

Ran an 8-angle finder / per-candidate verifier pass over the whole diff.
Confirmed findings were fixed in place; the rest are recorded as follow-ups.

**Fixed — correctness:**
- *Genesis topic routing* (`domain.rs`): remote ingest chose the group topic
  by CBOR-decoding the op body to detect `GroupCreated`; a body this binary
  can't decode (version skew) permanently mis-filed the genesis and the group
  went invisible. Now routes on the signed log-context prefix
  (`GROUP_GENESIS_CONTEXT_PREFIX`), with an append-side `ensure!` keeping the
  two sides in lockstep — no body decode.
- *Cross-author post censorship* (`groups/reduce.rs`): a `PostDeleted` for a
  not-yet-seen `post_id` tombstoned it on the deleter's self-claim, so a
  backdated foreign delete (author-signed ordering timestamp) silently
  dropped another member's post on every replica. Tombstones now record the
  deleter and only suppress a post by that same author. Regression test added.
- *Ordering-timestamp chaining* (`domain.rs`): `previous_ordering` was the max
  over *decoded* ops, which now skip-on-undecodable, so a reply to a newer
  peer's op could stamp before it. Switched to the header-only raw read paths
  (timestamp lives in the header) — chaining survives version skew.
- *Phantom "new activity" door* (`bridge.rs`): a member's own post/comment/
  heart bumped `latest_activity_at`, raising a digest door in their own river.
  Own-activity handlers now `mark_opened`.

**Fixed — performance / locking:**
- Group topic task ran the full duties+reduce+emit+suggestions fan-out per
  received op (O(ops²) on backlog catch-up); now gated to live mode, with the
  batch-completion arm folding the backlog once.
- Profile topic task recomputed hub suggestions on every non-local op; now
  gated to `GroupMembershipAdvertised` ops only (behaviour-preserving).
- `find_blob_secret` reduced every registered group's log per media fetch;
  now scoped to members-only groups (content kind backfilled into the
  registry when the genesis first reduces — the previously-unused `kind`
  column now earns its keep). Public groups never seal a blob.
- Group publish/edit held the global sync mutex across blocking
  attachment import+encrypt; now imports with the lock released, mirroring
  the profile path.

**Fixed — cleanups:** deduped `GLOBAL_GROUPS_CONTEXT_ID` to one `pub(crate)`
constant shared by the spaces and groups services (drift would fork the auth
state); built the `GroupCreated` genesis op once instead of two field-for-field
literals that could diverge.

**Also fixed — protocol flag day:** `GroupMembershipAdvertised` rides the
shipped `jyn/domain/v2` Contacts topic, and released clients hard-error on the
unknown variant (dropping an upgraded friend's whole reduction). Pre-1.0 and
released off this branch, so we took the flag day now: `DOMAIN_TOPIC_NAMESPACE`
`v2`→`v3` (old clients partition off and never receive the new variant) and
`DATA_SCHEMA_VERSION` `3`→`4` (local domain store wiped on upgrade so nothing
lingers on the retired topics). Identity, friend codes and settings survive;
posts/friendships re-form. Because the reducer now skips undecodable ops, later
additive variants won't need another bump.

**Deferred follow-ups (not blocking; recorded for later):**
- *Non-atomic ownership transfer*: promote-heir + demote-self are two ops with
  no atomicity; a crash between them strands two `Manage` holders. Recoverable
  with existing ops (either holder can demote the other) but not self-healing —
  wants an atomic `TransferOwnership` governance op or a reconcile self-heal.
- *Maintenance-tick triple reduction* / *JoinGroup all-groups peer sweep* /
  *`process_backlog` per-op `is_processed` N+1*: confirmed but bounded
  (startup / per-tick); a shared per-group reduction and a batched processed
  lookup would trim them.
- *Sleeps under the sync mutex* (contact-settle on JoinGroup and the tick):
  move the settle sleeps outside the lock / out of the user-facing handler.
- *Quality*: `GroupsIngestReport` is dead plumbing (every caller discards it);
  three Dart `nameOf` copies diverge and could share one helper (also fixes a
  `ref.read` staleness); `reconcile_group_auth`/`_space` share ~28 duplicated
  lines; the `unique_suffix`/`new_post_id` nonce recipe and the
  `normalize_profile_id` parse could be reused; the p2panda error-string
  matches (pinned rev, convention-consistent) could match typed variants.
