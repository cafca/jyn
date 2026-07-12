# Groups future direction and the alignment rules it imposes on phase one

**Status:** accepted

Phase one builds single-owner Groups
([ADR-0001](0001-single-owner-groups-first.md)). This ADR records where Groups
is heading and the rules that keep phase-one code cheap to evolve toward it.

## Roadmap (post phase one)

- **Shared multi-admin governance** — the committed fast-follow: multiple
  `Manage` holders, collective governance, epoch-fork handling.
- **Friends/Circles re-expressed as auto-derived Groups**
  ([ADR-0004](0004-groups-as-standalone-subsystem.md)); bespoke `JynSpaces`
  eventually retires.
- **Full ownership/management transfer to arbitrary users + total
  creator-independence** ([ADR-0006](0006-group-identity-and-metadata.md)).
- **Group as an outward-facing user-like identity.** To non-members a Group can
  appear *as if it were a user* — with its own profile and a public post stream
  visible to non-members. Those outward posts are authored by members via a
  mechanism **not yet decided**. (This is distinct from within-group posts,
  which are always member-authored and live in the group context.)
- **Multiple roles per member**, each role granting a set of actions.
- **Subgroups** — nested groups with their own visibility and governance.
- **Moderation tools** enforcing group rules.
- Other deferred: group deletion, content-mode migration, owner-only
  announcement Post mode, per-member advertisement opt-out, richer discovery.

## Alignment rules for phase one

Pre-shaped now (cheap now, costly to retrofit):

- **Authority is a role held by a member — and membership carries a *set* of
  roles**, never a hardcoded owner boolean. Every permission check routes
  through `roles → permitted-actions` (union over the member's held roles).
  Phase one populates only `Owner`={Manage}, `Member`={Write}.
- **Membership is an append-only log of join/leave/role ops**, never mutable
  boolean state.
- **Governance/membership operations are an extensible, versioned op set** (only
  `add-member` / `remove-member` / `edit-metadata` now) so moderation ops add
  without a schema break.
- **A post's presence *within* a group is separable from author-delete**, so a
  future moderation-hide op can act on it.
- **Content / Join / Discoverability are extensible data on the group record**,
  not hardcoded branches.
- **Nothing mutable is anchored to the creator** — only the GroupId derives from
  genesis.

Merely not-precluded (no structure built):

- **The Group is a first-class actor**, not modelled as an internal-only
  container — leave room for it to gain an outward user-like identity/profile
  and a group-attributed outward stream. Do not bake in "only individual users
  can be authors/profiles."
- **Group identity, membership, and topic stay per-group and composable** — do
  not assume groups are permanently flat, and do not bake "a member is always an
  individual key" so tightly that a group-as-member (subgroups) is blocked.
- **The Groups model must be able to express an auto-derived, blinded,
  always-encrypted per-profile group** — the litmus test for spaces becoming a
  special case of Groups.
