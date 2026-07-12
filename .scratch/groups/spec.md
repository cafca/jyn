# Spec: Groups (single-owner, phase one)

Status: ready-for-agent

Synthesizes the design settled in `CONTEXT.md` and `docs/adr/0001`–`0014`. Those
ADRs are the authoritative record of *why* each decision was made; this spec is
the *what* to build. Use the `CONTEXT.md` glossary vocabulary throughout
(Group, Owner, Member, Join mode, Content mode, Discoverability, GroupId, Group
place, Groups hub, Group admin, Digest door).

## Problem Statement

Today a person can only post into their own ambient personal stream (the river),
with a per-post visibility dial. There is no way to gather a set of people
around a shared, named place and post *into that place* — a container with its
own membership and its own fixed visibility. People want groups: some open and
public, some private and members-only; some you can walk into, some you must ask
to join. They also want to find the groups their friends are part of.

## Solution

Introduce **Groups**: hard containers a person creates and owns, that other
people join and post into. A Group has a fixed **Content mode** (public =
plaintext/world-readable, or members-only = encrypted to members) and a **Join
mode** (open = anyone joins, or request-to-join = the Owner approves). A Group
is reached from a dedicated **Groups hub** destination that lists the groups you
belong to and suggests groups your friends are in. Each Group has a **place**
screen with its stream and composer, and Owners govern from a dedicated **admin**
view. Posting into a Group is context-exclusive: the post lives in the Group
only, never in the author's river or profile. Group activity comes back to
members as a single **digest door** per group in the river.

This phase ships **single-owner** Groups — one Owner holds all authority —
built as a standalone subsystem designed so that shared multi-admin governance,
ownership transfer, and spaces-as-a-special-case are all additive later.

## User Stories

### Creating and owning

1. As a person, I want to create a Group from the Groups hub, so that I can gather people around a shared place.
2. As a Group creator, I want to set the Group's name, Content mode, Join mode, and Discoverability at creation, so that the Group behaves the way I intend from the start.
3. As a Group creator, I want to automatically become the Group's Owner, so that I control it without extra steps.
4. As an Owner, I want to edit the Group's name at any time, so that I can rename it as it evolves.
5. As an Owner, I want to change the Join mode (open ↔ request-to-join) at any time, so that I can open or gate joining as needed.
6. As an Owner, I want to change Discoverability (listed ↔ unlisted) at any time, so that I can make the Group findable or secret.
7. As an Owner, I want the Content mode to be fixed after creation, so that I never accidentally expose encrypted content or fail to protect it.
8. As a person, I want no limit on how many Groups I create or join, so that I can participate freely.

### Joining

9. As a non-member of an **open** Group, I want to join it directly, so that I can participate without waiting.
10. As a non-member of a **request-to-join** Group, I want to send a join request, so that the Owner can approve me.
11. As an Owner of a request-to-join Group, I want to see pending join requests, so that I can approve or decline them.
12. As an Owner, I want pending requests to be visible only to me, so that a declined request is never a public record.
13. As a person who requested to join, I want to see my own pending state, so that I know my request is outstanding.
14. As a joiner of a members-only Group, I want to gain read access once admitted, so that I can see the Group's content.
15. As a joiner, I want my join to complete once the Owner's node processes it, so that membership is authoritative even if the Owner was briefly offline.

### Posting and reading

16. As a Member, I want to post into a Group from its place, so that my post belongs to that Group only.
17. As a Member, I want the Group's fixed visibility to apply to my post automatically, so that I don't choose a per-post visibility inside a Group.
18. As a Member, I want to still choose my post's lifetime (ephemeral/permanent), so that I control how long it lasts.
19. As a Member, I want to edit, delete, and promote my own Group posts, so that I keep the same authorship control I have elsewhere.
20. As any person, I want to read a **public** Group's posts even without joining, so that I can browse it freely.
21. As a non-member, I want to be unable to read a **members-only** Group's posts, so that the Group's content stays confidential.
22. As a Member, I want to comment on Group posts, with comments protected the same way as the post (encrypted for members-only, plaintext for public), so that discussion matches the Group's confidentiality.
23. As a Member, I want to keep a Group post in my private collection under the usual lease semantics, so that I can retain it subject to the author's intent.
24. As a Member, I want media in Group posts to work like everywhere else (encrypted blobs for members-only, plaintext for public), so that photos, audio, and video behave consistently.

### Membership lifecycle

25. As a Member, I want to leave a Group at any time, so that I'm never trapped in it.
26. As an Owner, I want to remove a Member, so that I can enforce who belongs.
27. As an Owner of a members-only Group, I want a removed Member to be unable to read future posts, so that removal is meaningful.
28. As a Member removed from a members-only Group, I understand I keep only what I already received, so that expectations about clawback are honest.
29. As an Owner, I want to transfer ownership to another Member before I leave, so that the Group survives my departure.
30. As a person, I want the record of who was a Member and when to be preserved, so that "who could read, and from when" is auditable.

### Discovery

31. As a person, I want the Groups hub to list all Groups I'm a member of, so that I have one place to find them.
32. As a person, I want the Groups hub to suggest Groups my friends are members of but I'm not, so that I can discover relevant Groups.
33. As a Member of a **listed** Group, I want my membership advertised to my friends, so that they can discover the Group through me.
34. As a Member of an **unlisted** Group, I want my membership never advertised, so that a secret Group stays secret.
35. As a person, I want to learn only my own friends' memberships, never strangers', so that discovery respects the social graph.
36. As a person viewing a members-only Group's roster, I want it visible only if I'm a member, so that the member list is as protected as the content.
37. As a person viewing a public Group's roster, I want it visible to anyone, so that public Groups are transparent.

### The river and hearts

38. As a Member, I want one digest door per Group with new activity in my river, so that Groups don't flood my river with individual posts.
39. As a Member, I want the digest door to open the Group place, so that I can go read the activity.
40. As a person, I want a river door only for Groups I'm a member of, so that the river reflects my actual memberships.
41. As a person, I want to heart a public Group post and have it surface to my friends as a named discovery card pointing into the Group, so that I can spread things I like.
42. As a person, I want a heart on a members-only Group post to never reach non-members, so that confidentiality is never broken by propagation.
43. As a person, I want hearts on unlisted Groups to stay in-group, so that unlisted means no automatic outward surfacing at all.

### Places and administration

44. As a person, I want a Group place screen showing the Group's identity, stream, and (if I'm a member) composer, so that I have a home for the Group.
45. As a non-member at a public Group place, I want to read and see a Join/Request affordance, so that I can participate.
46. As a non-member at a members-only Group place, I want to see identity and a Join/Request affordance but no content, so that I can ask in without seeing protected content.
47. As an Owner, I want a dedicated admin view (not inline on the place), so that governance has its own clear surface.
48. As an Owner in the admin view, I want to edit metadata, approve/deny requests, remove members, and transfer ownership, so that I can fully govern the Group.

## Implementation Decisions

### Architecture

- Build a **standalone Groups subsystem** in the core, using p2panda primitives
  (`p2panda-auth`, `p2panda-encryption`) directly. Do **not** build on, extend,
  or entangle with the existing per-profile `JynSpaces` module, which is left
  untouched. Some near-term duplication of crypto plumbing is accepted; the
  intended endgame is that Friends/Circles become auto-derived Groups and
  `JynSpaces` retires. (ADR-0004)
- **Single-owner, role-based authority.** The Owner holds the sole `Manage`
  role; Members hold `Write`. Authority is always a role held by a member, never
  a hardcoded owner boolean or a field on the Group record. (ADR-0001)

### Data model

- **GroupId** = the hash of the Group's creation (genesis) op. Stable across
  ownership transfer; also derives the Group's replication topic. (ADR-0006)
- **Group record** carries: display name, Content mode (`public` |
  `members_only`, immutable after creation), Join mode (`open` | `request`,
  mutable), Discoverability (`listed` | `unlisted`, mutable). Stored as genesis
  payload plus later metadata-update ops by the `Manage` holder. Content/Join/
  Discoverability are extensible data on the record, not hardcoded branches.
  (ADR-0002, ADR-0006, ADR-0008)
- **Membership is an append-only log of ops** (join, leave, remove, role change,
  metadata edit), never mutable boolean state — so the read-eligibility timeline
  is always derivable. Each membership entry carries a **set of roles** (phase
  one populates only `Owner`={Manage}, `Member`={Write}); permission checks
  route through a `roles → permitted-actions` function (union over held roles),
  never `if owner`. (ADR-0002, ADR-0014)
- **Governance/membership operations are an extensible, versioned op set** —
  only `add-member` / `remove-member` / `edit-metadata` exist now; the enum must
  accept future moderation/role ops without a schema break. A post's presence
  *within* a Group is modelled separably from author-delete, so a future
  moderation-hide op can act on it. (ADR-0014)
- **Nothing mutable is anchored to the creator.** Mutable group state
  (membership, metadata, seeding) anchors to the *current* `Manage` holder and
  the member set. Only the GroupId derives from genesis. The Group must survive
  the creator's total departure after ownership transfer. (ADR-0006)

### Encryption / auth split

- **Every Group has an auth group** (`p2panda-auth`) for membership and roles,
  in *both* Content modes. This yields roster, join/leave/remove ops, and
  transfer-of-`Manage`. (ADR-0002)
- **Members-only Group** = additionally a full encrypted space: Group posts,
  comments, and media are encrypted to the Group. Removing/leaving a member
  triggers **lazy re-key** (re-key right before the next Group post); content
  already delivered is not clawed back. (ADR-0002, ADR-0003)
- **Public Group** = auth group only, no encryption: membership governs posting
  rights and roster; posts publish as plaintext tagged with the GroupId.
  (ADR-0002)

### Joining and authority

- **Join mode is a policy on the Owner's node**, not a change to who holds
  authority. Open = the Owner's client auto-accepts join requests; Request =
  the Owner's client surfaces them for manual acceptance. Membership only ever
  mutates via the Owner. The Owner's node must be online to admit anyone
  (accepted cost). (ADR-0005)

### Replication

- **Each Group is its own replication topic derived from GroupId** — a new
  replication axis alongside the per-profile friend-circle topics. Members (and,
  for public Groups, readers) join the Group topic; the Owner's
  membership-control ops and every member's Group posts replicate under it,
  regardless of friendship. Members are the creator-independent seeding set.
  Group posts are authored on the member's own log but replicated via the Group
  topic only — never onto the author's friend-facing profile topic, so they
  never appear in the author's river or profile. (ADR-0007)

### Discovery

- **Roster visibility follows Content mode** (public → public roster;
  members-only → members-only). This is separate from **membership
  advertisement**: a member's `listed` Group memberships are added to the same
  friend-visible profile state that already carries follow lists, so friends can
  aggregate and suggest them. `unlisted` = no advertisement and no outward
  heart-surfacing. (ADR-0008, ADR-0009)

### Propagation (hearts)

- **Outward heart-discovery happens iff Content mode = Public AND Discoverability
  = listed**; the discovery card points into the Group context. Otherwise a
  heart is a purely in-group named like. Members-only never leaks. (ADR-0009)

### Core interface (the seam)

- Extend the existing **`AsyncBridge` command/event boundary** (`NetworkCommand`
  in, `NetworkEvent` out) — the same interface the UI uses and the integration
  tests drive. New commands cover: create Group, join, request-to-join, approve/
  deny request, post to Group, edit metadata, remove member, transfer ownership,
  leave. New events cover: Group state/metadata, membership roster, pending
  requests, per-Group digest, and discovery suggestions. Keep group behavior
  observable entirely at this boundary.

### UI (Flutter)

- **Groups hub** — a dedicated top-level destination: lists member Groups,
  shows friend-based suggestions, hosts Create-group. (ADR-0012)
- **Group place** — per-Group screen: header (identity + viewer status), the
  Group stream, and an in-group composer for members (no visibility dial,
  lifetime retained). Adapts to viewer state (non-member/member/owner).
  (ADR-0013)
- **Group admin** — dedicated Owner-only sub-view reached from the place: edit
  metadata, approve/deny requests, remove members, transfer ownership.
  (ADR-0013)
- **Digest door** — one river entry per member-Group with new activity, opening
  the place. (ADR-0010)
- Screens render off Riverpod providers fed by bridge events.

## Testing Decisions

- **A good test verifies external behavior through a public interface, not
  implementation details** — it reads like a specification and survives
  refactors. For Groups, "external behavior" means what an actor observes at the
  bridge, over a real multi-node network.
- **Primary seam — the `AsyncBridge` command/event boundary.** All Group
  behavior is proven here, with multi-node (Alice/Bob/Carol) setups over a real
  relay. This exercises the whole core (Groups subsystem, auth, encryption,
  sync) exactly as the UI does. Marquee tests:
  - members-only reach + lazy re-key on removal (prior art: `core/tests/circles.rs`);
  - join pending until the Owner's node processes it, including an offline Owner
    (prior art: `core/tests/friendship.rs`'s offline-target case);
  - public vs members-only read access for non-members;
  - membership advertisement: a friend sees a `listed` Group in suggestions; an
    `unlisted` one never surfaces;
  - ownership transfer, then the former Owner leaves, and the Group persists.
- **Secondary seam — the Riverpod provider boundary** for the Flutter surfaces
  (hub, place, admin, composer, digest door), fed by bridge events. Prior art:
  `app/test/providers_test.dart`. No widget-level pixel-driving tests.
- **Modules tested:** the Groups subsystem and bridge command/event handling
  (via the bridge seam); the Groups Flutter providers (via the provider seam).

## Out of Scope

- **Shared multi-admin governance** (multiple `Manage` holders, collective
  governance, epoch-fork handling) — the committed fast-follow, not this phase.
- **Group deletion** and, with it, the clean sole-owner exit — deferred; a
  sole-owner Group becomes dormant for now. (ADR-0003)
- **Content-mode migration** (public ↔ members-only after creation). (ADR-0006)
- **Subgroups / nesting** — not built; the model must merely not preclude it.
- **Moderation tools** — not built; only the op-set extensibility is reserved.
- **Roles beyond Owner/Member** — the role-set indirection is built, but only
  two roles are populated.
- **Owner-only "announcement" Post mode** — every Member can post. (ADR-0002)
- **Per-member advertisement opt-out** — advertisement is group-level only.
  (ADR-0008)
- **The Group as an outward-facing user-like identity** with its own public
  stream — not built; the model must merely not preclude the Group being a
  first-class outward actor. (ADR-0014)
- **Share codes / invite links** — not implemented at all in this phase.
  Discovery is the Groups hub + friend-based suggestions only. Share codes may
  be added later as an out-of-band join path; nothing about them is built now.
- **Richer discovery** (search/browse beyond friend-based suggestions).
- **Heavier widget-level UI tests** — provider-level only.

## Pre-merge checks

- **Story 28 (removal clawback) vs. encryption phase 3 (garbage collection).**
  Story 28 currently promises "a removed member keeps only what they already
  received; no clawback." This is tied to whether the encryption plan's **phase
  3 — garbage collection of expired ciphertext + blobs** has landed. **At the
  very end, before merging this work, check whether encryption phase 3 (GC) is
  on `main`.**
  - **If phase 3 (GC) has landed on `main`:** GC-based handling of a removed
    member's retained/expired content becomes **in scope** for this work —
    revisit story 28's contract and removal semantics so removal can actually
    reduce retained content via GC-driven expiry, and implement accordingly.
  - **If phase 3 has not landed:** leave story 28 as-is (honest no-clawback
    contract) and record the GC-based tightening as a **follow-up**, not part of
    this merge.

## Further Notes

- **Open design item the implementer must resolve or escalate:** the exact
  **key-delivery / re-key wire mechanism** for members-only Groups was settled
  at the *model* level (single-admin auth group, lazy re-key on removal, no
  clawback) but **not** at the byte/protocol level. This is the one part of the
  spec that still needs protocol design before the members-only path is
  implementable; treat it as the first thing to nail down.
- The **group-as-outward-user authoring mechanism** is explicitly undecided and
  out of scope; just don't foreclose it. (ADR-0014)
- **Litmus test for the subsystem's generality:** an auto-derived, blinded,
  always-encrypted, per-profile Group should be *expressible* in the Groups
  model even though it isn't built — this is what will later let Friends/Circles
  become a special case of Groups. (ADR-0004, ADR-0014)
- The alignment rules in ADR-0014 (role-as-set, append-only membership,
  extensible op set, nothing anchored to the creator, first-class outward
  actor, composable identity) are load-bearing: they are what keep the deferred
  roadmap cheap. An implementation that violates them is wrong even if it passes
  the phase-one tests.
