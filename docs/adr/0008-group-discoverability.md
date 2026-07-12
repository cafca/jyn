# Group discoverability: roster visibility vs. friend-facing membership advertisement

**Status:** accepted

Groups must be discoverable through friends ("which groups are my friends in?")
without violating the members-only roster rule
([ADR-0002](0002-group-join-and-content-modes-independent.md)). These are two
different relations, kept separate:

- **Roster visibility** (group-controlled) — enumerating the *full* member
  list. Unchanged: follows Content mode.
- **Membership advertisement** (friend-facing) — a member disclosing *their
  own* edge ("I'm in G") to *their own friends*. Reveals one member + the
  group's existence/id, never the roster. A user only ever learns their own
  friends' memberships.

## Decisions

- **Discoverability is a third Group property, Owner-set: `listed` | `unlisted`**
  (orthogonal to Join mode and Content mode). Default `listed`. `unlisted` = a
  secret group: no membership advertisement, link-only. This is the safety
  valve for a sensitive members-only + request-to-join group.
- **Control is group-level only in this phase.** In a `listed` group every
  member is advertised to their friends. Per-member opt-out ("hide my
  membership") is deferred as an additive follow-up. Matches the existing "friend
  lists are mandatorily friend-visible, no opt-out" precedent.
- **Mechanism — ride the friend-visible profile state.** A member's `listed`
  group memberships are added to the same friend-replicated profile state that
  already carries follow lists (which `derive_circle_members` reads). Discovery
  UIs aggregate friends' advertised groups. The exact suggestion UX is
  deferred; this only guarantees the data exists.
