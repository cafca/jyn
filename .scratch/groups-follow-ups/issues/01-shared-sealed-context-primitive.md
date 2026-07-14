# 01 — One sealed-context primitive for circles and group encryption

**Status:** needs-triage

**Context:** `core/src/spaces/` (profile circles/friends encryption) and
`core/src/groups/service.rs` (members-only groups) are two parallel
implementations of the same idea: a p2panda-spaces `Manager` over the shared
sqlite store, sealing CBOR `DomainOperation`s into `Spaces` wrappers on a
topic. Requested follow-up from the ADR-0018 review conversation.

## Problem

The two stacks duplicate, member for member:

- a `Forge` (`spaces/forge.rs::JynForge` vs `groups/service.rs::GroupsForge`)
  that wraps `SpacesArgs` into a `DomainOperation::Spaces`, appends it, and
  pushes an outbox entry;
- a **placement hint** (`PlacementHint`) parked before `space.publish` so the
  opaque wrapper lands in its inner post's expiry bucket (ADR-0016/0018);
- a publish path (`JynSpaces::publish_encrypted` vs
  `JynGroups::encrypt_to_group`): pre-publish repair/reconcile, seal,
  persist manager state, `store_decrypted_inner_operation`, `mark_processed`;
- a pending-message queue with retry (`PendingMessage` vs
  `PendingGroupsMessage`), `drain_pending`, and a `process_backlog` that
  re-scans raw operations after restart;
- shared-state couplings that already had to be patched pointwise so the two
  copies don't drift: `credentials_for`, `GLOBAL_GROUPS_CONTEXT_ID`, and the
  single `ops_lock` serializing both managers over one auth graph.

Every fix lands twice or silently misses one side — the ADR-0018 placement
hint had to be re-implemented for groups even though the mechanism was
identical, and the review found the two `process_backlog`s share the same
N+1 shape (issue 06).

## Fix direction

Extract a `SealedContext` (name open) owning: manager + forge + placement
hint + pending queue + backlog scan + repair/reconcile hooks + outbox, over
an audience id and a `SpacesStore`. `JynSpaces` becomes two instances
(friends, circles) plus profile-specific policy (audience derivation, space
kind blinding); `JynGroups` becomes one instance per members-only group plus
group policy (auth mirroring, owner duties). The `ops_lock` and key registry
stay shared by construction instead of by convention. Behavior-preserving
refactor; the existing groups/circles integration suites are the safety net.
