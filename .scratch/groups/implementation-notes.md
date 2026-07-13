# Groups implementation notes (working doc)

Status: complete. Companion to `spec.md`; records the concrete mapping of the
spec onto this codebase, settled while implementing. All four tickets are
landed, reviewed and verified. Two spec stories are intentionally out of
scope (decisions, not gaps): story 28 (clawback) ships as honest no-clawback
until phase-3 retention lands on `main`, and story 23 (keep-a-group-post) is a
recorded follow-up — both noted in the ticket comments.

## Wire / domain layer

- **Group topic**: `group_sync_topic(group_id)` = hash of
  `b"jyn/groups/v1/" + group_id`. A new replication axis (ADR-0007); profile
  topics are untouched.
- **Group logs**: `DomainLogId { profile_id: <group-scoped context id>, kind }`
  — the `profile_id` field doubles as a generic log-context id. Each author
  gets one `DomainLogKind::Groups` log per group (`profile_id` = the GroupId
  string). The genesis op lives on a one-op log whose context id is a random
  nonce (`jyn/group-genesis/<unique>`), because the GroupId is the hash *of*
  the genesis op and so cannot appear in its own header. Both log families are
  associated with the group topic.
- **New `DomainOperation` variants** (only ever on group topics, except the
  ticket-02 advertisement op):
  - `GroupCreated { creator_profile_id, name, content_mode, join_mode, discoverability, created_at }`
    — genesis. GroupId = this op's hash; members validate that equality.
  - `GroupGoverned { group_id, action, recorded_at }` — authored by the
    current `Manage` holder. `action` is the extensible, versioned
    `GroupGovernanceAction` enum: `AddMember { member, roles }`,
    `RemoveMember { member }`, `SetMemberRoles { member, roles }` (ownership
    transfer = promote new owner, demote old), `EditMetadata { name?, join_mode?, discoverability? }`.
  - `GroupJoinRequested { group_id, requester_profile_id, requester_display_name, greeting?, recorded_at }`
    — foreign-authored on the group topic (same pattern as
    `FriendshipRequested` on a profile topic). Serves both join modes; in Open
    mode the Owner's node auto-accepts it.
  - `GroupLeft { group_id, member_profile_id, recorded_at }` — self-authored,
    takes effect in reduction immediately (no owner liveness needed).
  - Group posts / edits / deletes / lifetime changes / comments / hearts reuse
    the existing post + interaction variants, appended to the author's group
    log. Public-group posts carry `Visibility::Public`.
- **Reduction**: `read_group_state(group_id)` reduces a group topic's ops to
  `ReducedGroupState` — metadata, roster (each entry a *set* of `GroupRole`s),
  pending join requests, membership timeline, posts with comments/hearts
  joined. Authorship rules mirror the profile reducer: genesis only from the
  op whose hash is the GroupId; governance only from the `Manage` holder *at
  that point in the log*; leave only self-authored; posts/comments/hearts only
  from members holding `Write` at that point. Permission checks route through
  `permitted_actions(roles)` — never `if owner`.
- Deny of a join request is a **local-only** record on the Owner's node
  (ADR-0002: a declined request is never a public record).

## Auth layer (p2panda-auth / p2panda-spaces)

- Reduced domain state is the source of truth for the roster/UI (same pattern
  as friends-list → spaces reconciliation today). The p2panda-auth group is
  the crypto substrate, **reconciled** to the reduced roster by the Owner's
  node:
  - public group → `manager.create_group` (auth-only; no key bundles needed),
  - members-only group → `manager.create_space(GroupId)` (ticket 03), whose
    welcome-on-add delivers the group secret; lazy re-key before the next
    post via the same repair/remove-stale flow as circles.
  - transfer = auth `Promote`/`Demote` via `Group::process_local_control`.
- The Groups subsystem has its **own `Manager`** with a forge that appends
  spaces messages to *group* logs (topic = the group's), sharing the sqlite
  store and one operations lock with `JynSpaces` (which is left untouched
  apart from exposing the shared lock). Shared store = shared key registry,
  so a joiner's key bundle processed from their profile topic is visible to
  the groups manager.

## Node behaviour

- Members (and public readers) join the group topic via LogSync, like contact
  topics; a per-group topic task ingests ops, feeds the groups service, and
  re-emits `GroupStateUpdated`.
- The Owner's node is the only admission path (ADR-0005): it processes
  `GroupJoinRequested` (auto in Open mode, on command in Request mode),
  appends `AddMember`, and reconciles the auth layer. Startup backlog +
  maintenance re-runs cover the offline-owner case.
- Digest doors: the runtime tracks last-opened per group in a local store;
  a river door appears for member groups whose latest activity is newer.

## Bridge seam additions

Commands: `CreateGroup`, `JoinGroup` (sends the join request; also used for
request-to-join), `ApproveGroupRequest`, `DenyGroupRequest` (local),
`PublishGroupPost`, `EditGroupPost`, `DeleteGroupPost`,
`SetGroupPostLifetime`, `EditGroupMetadata`, `RemoveGroupMember`,
`TransferGroupOwnership`, `LeaveGroup`, `SetGroupHeart`,
`PublishGroupComment`, `KeepGroupPost`, `SyncGroup` (visit / read-only),
`MarkGroupOpened`.

Events: `GroupStateUpdated { group_id, state }` (metadata, viewer status,
roster when visible, pending requests for the owner, posts), plus hub/door
data folded by the runtime into `JynEvent` (`Groups`, `GroupPlace`, river
digest doors, group discovery cards).

## Decisions settled while implementing

- `manager.create_group(&[])` creates an **empty** auth group (unlike
  `create_space`, no auto-creator); the creator is passed explicitly with
  `Access::manage()`.
- The pinned crate exposes no promote/demote on the group API
  (`visible_cone` is crate-private), so auth-mirror **access changes go
  remove-then-re-add during reconcile**, and reconciliation never touches the
  local actor's own auth membership — after a transfer, the incoming `Manage`
  holder's node converges the old owner's access. The jyn domain log stays
  the authoritative audit trail either way.
- Both managers (JynSpaces, JynGroups) share the sqlite store, the processed
  set, and one ops lock; group control messages ride group logs via the
  GroupsForge, whose auth-id ↔ GroupId binding comes from a pending-create
  slot (own groups) or from message placement on the group's log (members).

- Members-only key exchange rides profile topics (ADR-0015's flow): the
  Owner syncs each joiner's profile topic (their key bundle lives on its
  Spaces log) and members sync the Owner's — `sync_group_peer_profiles`,
  run at startup, on join/respond commands, and each maintenance tick.
  Space membership mirrors the reduced roster: adds eager (welcome delivers
  the secret), removals only in the pre-publish lazy re-key.

## Honest caveats

- Wire-level roster blinding for members-only groups is impossible with the
  reused protocol (auth control messages are plaintext, as they are for the
  friends space today); roster confidentiality is enforced at the bridge/API
  surface (non-members receive no roster), matching ticket 03's test at the
  seam.
