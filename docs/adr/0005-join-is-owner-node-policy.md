# Join mode is a policy on the Owner's node; membership always mutates via the Owner

**Status:** accepted

Single-admin auth ([ADR-0001](0001-single-owner-groups-first.md)) means only the
Owner (`Manage` holder) can change group membership. Open join
([ADR-0002](0002-group-join-and-content-modes-independent.md)) therefore cannot
mean self-service. Instead:

- **Open** = the Owner's client **auto-accepts** join requests and appends the
  add-member op to the Owner's log.
- **Request-to-join** = the Owner's client surfaces the request for **manual**
  acceptance.

Membership only ever mutates via the Owner, in both modes. Join mode is a
policy toggle on the Owner's node, not a change to who holds authority.

## Accepted cost

The Owner's node must be online to admit anyone. In Open mode a join is
*pending* until the Owner's node processes it; for a Members-only group the
joiner cannot read until the Owner has admitted them and delivered keys. There
is no server or relay to accept on the Owner's behalf — such a relay would be a
second admin, which is the deferred multi-admin (B) world.

## Rejected alternative

Self-add join ops for Open groups (a member appends their own membership op).
Removes the liveness dependency but breaks single-admin auth, forks the model
between Open and Request groups, and complicates the multi-admin transition.
