# Group activity surfaces as one river digest door per member-group; no follow relation

**Status:** accepted

- **One digest door per group with new activity** — not one river item per group
  post ([ADR-0007](0007-group-replication-topic.md) already keeps group posts
  out of the interleaved river). The door summarizes recent activity and opens
  the group **place screen**; it sorts into the reverse-chron river by the
  recency of the group's latest activity.
- **River doors require membership.** The only relation is membership; we do
  **not** introduce a separate "follow/subscribe to a group" relation. To get a
  group in your river you join it (frictionless for open groups; for
  request-to-join, approval gates river-presence just as it gates posting).
- **Non-member reading of a public group is visit-only** — you open its place
  screen, or arrive via a heart's discovery card
  ([ADR-0009](0009-heart-propagation-from-groups.md)). No river door without
  membership.
