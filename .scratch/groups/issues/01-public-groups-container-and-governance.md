# 01 — Public Groups, end to end (container + governance)

**What to build:** The complete public-Group experience. A person can create a
public Group, other people can join and post into it, and Owners govern it —
all with plaintext content and no encryption. This is the container + governance
skeleton the rest of the feature builds on. Behaviour is proven at the
`AsyncBridge` command/event seam with multi-node (real-relay) tests; Flutter
surfaces are tested at the Riverpod provider level. Use the `CONTEXT.md`
glossary and respect ADRs 0001–0014.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] A person can create a Group from the Groups hub, setting name, Join mode (open | request-to-join), and Discoverability (listed | unlisted); Content mode is fixed to `public` at creation and thereafter immutable.
- [ ] The GroupId is the hash of the creation (genesis) op; the new Group appears in the creator's Groups hub with the creator as Owner and sole Member.
- [ ] Membership and roles are a single-owner `p2panda-auth` auth group: Owner holds the sole `Manage` role, Members hold `Write`. Authority is a role held by a member (never a hardcoded owner boolean); each membership entry carries a *set* of roles; permission checks route through a `roles → permitted-actions` function.
- [ ] Membership is an append-only log of ops; the governance/membership op set is an extensible, versioned enum containing (for now) add-member / remove-member / edit-metadata.
- [ ] The Group has its own replication topic derived from GroupId; the Owner's membership-control ops and members' Group posts replicate under it across members regardless of friendship; members are the seeding set.
- [ ] A Member can post into a Group from its place (composer has no visibility dial; lifetime is a per-post choice); the post replicates via the Group topic only and never appears in the author's river or profile.
- [ ] A second node that is a member reads the Group's posts; a non-member can also read a public Group's posts.
- [ ] Open join: a non-member joins directly; the Owner's node auto-accepts and appends the add-member op. Request-to-join: the request is surfaced to the Owner for manual accept/deny; pending requests are visible only to the Owner; the requester sees their own pending state.
- [ ] Joining is authoritative once the Owner's node processes it, including when the Owner was briefly offline (join stays pending until processed).
- [ ] The Group place adapts to viewer state (non-member sees Join/Request; Member sees composer + Leave; roster follows Content mode). Owner governance lives in a dedicated Group admin sub-view: edit name / Join mode / Discoverability, approve/deny requests, remove members, transfer ownership.
- [ ] A Member can leave; the Owner can remove a Member; the Owner can transfer `Manage` to another Member and then leave, and the Group persists (nothing mutable is anchored to the creator).
- [ ] Members see one river digest door per Group with new activity, opening the Group place; group posts never interleave individually into the river; a river door requires membership.
- [ ] Comments and media attachments work on public Group posts as plaintext, inheriting the Group's Content mode.
- [ ] Integration tests at the `AsyncBridge` seam cover create → post → cross-node read, open join, request-to-join with offline Owner, remove, and transfer-then-leave. Flutter hub/place/admin/composer are covered at the provider level.
