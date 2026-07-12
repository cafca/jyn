# Group content replicates via a per-GroupId topic, decoupled from friend-circle sync

**Status:** accepted

Today replication is per-profile and friend-circle: a node joins its own topic
and one topic per *friend* (`core/src/sync.rs`). A Group spans non-friends, so
friend-circle sync cannot carry group traffic.

- **Per-GroupId topic — a new replication axis.** Each Group is its own
  replication topic derived from its GroupId. Members (and, for public groups,
  readers) join the group topic; all group traffic — the Owner's
  membership-control ops and every member's group posts — replicates under it,
  across members **regardless of friendship**.
- **Members are the creator-independent seeding set.** Seeding follows the
  member set (and current Owner), never the creator specifically — satisfying
  the [ADR-0006](0006-group-identity-and-metadata.md) creator-independence rule;
  the group survives the creator leaving. For public groups, non-member readers
  also join the topic to read, but members are the durable seeders.
- **Group posts stay exclusively in the group context.** A group post is
  authored on the member's own log (author owns their data) but replicated via
  the group topic only — never onto the author's friend-facing profile topic.
  It therefore never appears in the author's river or on their profile.
  (Context is exclusive: `docs/2026-07-04-screen-design-foundations.md`.)
