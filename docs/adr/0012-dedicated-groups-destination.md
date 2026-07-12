# A dedicated Groups destination (hub), not a profile appendage

**Status:** accepted

Groups get their own top-level destination — the **Groups hub** — reachable
from primary navigation, rather than hanging off the profile screen. On it:

- **My groups** — all groups the user is a member of.
- **Suggestions** — groups the user's friends are members of but the user is
  not, aggregated from friends' `listed` membership advertisements
  ([ADR-0008](0008-group-discoverability.md)). This pulls friend-based
  discovery into scope for this phase (the *hub* is the discovery surface; the
  exact ranking/curation of suggestions can still evolve).
- **Create group** — the create action lives here.

## Deliberate deviation

The v1 design principle was four screens and "no engine-room screen, no new
destinations" (`docs/2026-07-04-screen-design-foundations.md`). A dedicated
Groups hub is a conscious post-v1 addition the owner chose over hanging groups
off the profile. Recorded so the minimalist principle isn't treated as
violated by accident.

## Consequences

- Adds a top-level destination alongside the river and profile; the exact
  navigation chrome (tab, door) is a design detail for the spec.
- The per-Group **Group place** screen (ADR-0013) is separate from this hub:
  the hub lists / discovers / creates groups; the place is a single group's
  stream + composer + governance.
