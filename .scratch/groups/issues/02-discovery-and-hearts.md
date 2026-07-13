# 02 — Discovery + hearts

**What to build:** People can discover Groups through their friends, and hearts
on public Group posts spread them the way public-post hearts already do. Adds
the Discoverability property, friend-based suggestions in the Groups hub, and
heart propagation for Groups. Proven at the `AsyncBridge` seam (multi-node) and
provider level. Respect ADR-0008 and ADR-0009.

**Blocked by:** 01.

**Status:** ready-for-human

- [x] Discoverability (`listed` | `unlisted`) is an Owner-set Group property, editable anytime, independent of Join mode and Content mode; default `listed`.
- [x] A Member's `listed` Group memberships are published in the same friend-visible profile state that already carries follow lists; `unlisted` memberships are never advertised.
- [x] The Groups hub suggests Groups that the viewer's friends are members of (via those advertisements) and that the viewer has not joined; a person only ever learns their own friends' memberships, never strangers'.
- [x] Roster visibility follows Content mode (public Group → roster and join/leave history visible to anyone); this is kept distinct from membership advertisement.
- [x] Hearting a post in a Public + `listed` Group surfaces a named discovery card on the liker's friends' rivers, framed with provenance and pointing into the Group context (the post is not copied or moved).
- [x] Hearts in a Public + `unlisted` Group stay in-group (no outward discovery card); i.e. outward heart-discovery happens iff Content mode = Public AND Discoverability = `listed`.
- [x] Integration tests at the `AsyncBridge` seam cover: a friend sees a `listed` Group in suggestions; an `unlisted` Group never surfaces; a public+listed heart reaches a friend's river as a discovery card; a public+unlisted heart does not.

## Comments

Implemented 2026-07-12. `GroupMembershipAdvertised` rides the Contacts log
(the friend-visible profile state); `reconcile_group_advertisements` keeps it
aligned (member × listed), rerunning from group commands, startup, and the
maintenance tick. Suggestions aggregate own friends' advertisements minus own
memberships and emit as `GroupSuggestionsUpdated`. Hearts carry optional
group context on the profile log iff Public+listed (retraction also covers a
since-unlisted group); the river materializes them as `GroupDiscoveryCard`s
(member groups filtered out), never as ghosts. Proven by
`core/tests/groups_discovery.rs` plus reducer/state unit tests and provider
tests; hub suggestions and river heart-cards are wired in the Flutter UI.
