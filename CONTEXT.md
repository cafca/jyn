# jyn

A peer-to-peer social app (Rust core + Flutter UI) built on p2panda. Ambient
personal streams ("the river") with per-post visibility and lifetime; encrypted
non-public content via sender-owned audience spaces.

## Language

### Groups

**Group**:
A hard container that people post into, with a fixed visibility of its own and
its own composer and place screen — distinct from the ambient personal stream.
A post created in a Group belongs to that Group only. Post-v1 feature.
_Avoid_: space (that is the encryption primitive), channel, room.

**Owner**:
The single member of a Group who holds the `Manage` role and governs
membership. Modelled as an assignable role, not a fixed property, so that
multiple `Manage` members become possible later without a rewrite. See
[ADR-0001](docs/adr/0001-single-owner-groups-first.md).
_Avoid_: admin (reserve for when multi-admin exists), creator.

**Member**:
A person who belongs to a Group. The Owner is also a Member (the one holding
`Manage`).
_Avoid_: participant, subscriber.

**GroupId**:
The stable identifier of a Group — the hash of its creation (genesis) op.
Survives ownership transfer. Also derives the Group's replication topic.
_Avoid_: group name (mutable, not an identifier).

**Join mode** (a Group property, independent of Content mode):
How a person becomes a Member. Either **Open** (anyone can join, no approval) or
**Request-to-join** (a prospective member asks; the Owner approves or declines).
_Avoid_: invite-only (not a chosen mode).

**Group post**:
A post authored into a Group. Same single post type as elsewhere. Its
**visibility** is fixed by the Group's Content mode (no per-post visibility
dial); its **lifetime** (ephemeral/permanent) remains a per-post author choice;
edit/delete/promote carry over unchanged.

**Content mode** (a Group property, independent of Join mode):
The fixed visibility of a Group's posts. Either **Public** (plaintext, readable
by anyone) or **Members-only** (encrypted to the members; unreadable to
non-members and passive peers). This is the "fixed visibility of their own" the
design doc gives Groups in place of per-post visibility.
_Avoid_: private (ambiguous with the per-post `Private` lifetime/visibility).

**Discoverability** (a Group property, Owner-set):
Either **listed** (members may advertise their membership to their friends, and
public-group hearts surface outward) or **unlisted** (secret: no automatic
outward surfacing by any mechanism). Independent of Join mode and Content mode.

**Membership advertisement**:
A member disclosing their *own* membership edge ("I'm in G") to their *own*
friends, via the friend-visible profile state. Distinct from roster visibility
(the full member list, governed by Content mode).

**Groups hub**:
The dedicated top-level destination listing the Groups you're a member of,
suggesting groups your friends are in, and hosting Create-group.

**Group place**:
The in-app screen for a single Group — its stream, its composer, and its
membership affordances. Reached from a river digest door or the Groups hub.

**Group admin**:
The dedicated Owner-only sub-view (reached from the Group place) for governing a
Group: edit metadata, approve/deny requests, remove members, transfer
ownership.

**Digest door**:
A single river entry per member-Group with new activity, summarizing it and
opening the Group place. Group posts never interleave individually into the
river.

### Existing (pre-Groups) vocabulary

**Space**:
The sender-owned, single-admin `p2panda-spaces` encryption group that backs an
audience. Every user owns their own Friends space and Circles space. An
encryption primitive, not a user-facing container.

**Circle**:
A user's friends-of-friends audience, auto-derived as `⋃(each friend's
friends)`. Backed by a Space.

**River**:
Home — the flowing, interleaved surface where posts pass by.
