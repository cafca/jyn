# Group logs are expiry-bucketed co-deletion units, scoped per group

**Status:** accepted

[ADR-0016](0016-logs-are-expiry-keyed-co-deletion-units.md) reframed the
p2panda log as a co-deletion unit: a log holds exactly the operations that
live and die together, placement computes the bucket, and GC drops drained
buckets whole. This ADR extends that structure into the group domain — group
content gets the same bucketing, scoped per group so buckets stay on the
group's topic.

## Placement

An author's operations for one group split across contexts, all carrying the
GroupId as their `audience` (so every log rides the group topic):

| Operations | Registry context | Lifetime |
| --- | --- | --- |
| Genesis, governance, join/leave, spaces control (auth, membership, re-key) | `group/<GroupId>/control` | permanent |
| `PostPublished` | `group/<GroupId>/bucket/<tier>/<window>` via `LogBucket::place` | dropped whole at window end |
| `PostEdited` / `PostDeleted` / `PostRehomed` | the post's current bucket | with their post |
| Hearts and comments | `group/<GroupId>/react/<month>` via `LogBucket::place_reaction` | reaped reactively, month dropped when drained |

The `LogBucket` math is reused unchanged; only the context string is
group-prefixed. This bounds log count by *active time windows*, not post
count (reconciliation overhead), while keeping each bucket cleanly deletable
— the two failure modes this design avoids are one-log-per-post (log
explosion) and everything-in-one-log (never deletable).

The **control log** is the one deliberately permanent log per author per
group: membership history must outlive any post. GC structurally never drains
it — governance ops classify as live.

## Members-only groups

An encrypted group post is an opaque `Spaces` wrapper by the time the groups
forge appends it, so the bucket is computed from the *inner* operation before
sealing and handed to the forge as a placement hint — the exact mechanism
ADR-0016 uses for profile spaces (`JynSpaces::publish_encrypted`). Control
traffic takes no hint and lands on the group control log.

Limit: GC decides drainability by reading payloads, and a member's device
classifies wrappers through its decrypted cache — so members reclaim expired
encrypted buckets, but a **non-member** holding a members-only group's
ciphertext sees only opaque wrappers, which deliberately keep their log
alive. Un-welcomed devices therefore retain such ciphertext indefinitely
(tracked: `.scratch/groups-follow-ups/issues/02-non-member-group-ciphertext-never-gcd.md`;
window-bound dropping without payload reads is the candidate fix).

## Lifetime changes re-home, like profile posts

Changing a group post's lifetime publishes a `PostRehomed` marker into the
old bucket (disowning the old copy so its media is not reclaimed as dead)
followed by a self-contained `PostPublished` snapshot into the new bucket.
The group reducer already treats the newest publication for a post id as its
current state; a tombstone still beats any later snapshot.

## GC

Because group topics share the audience-keyed topic derivation, the existing
GC primitives work on a group topic without change:

- `drop_drained_buckets(group_id, …)` prunes drained buckets of *every*
  member on the group topic (recipient-side reclamation included) and
  forgets our own dropped contexts so retired ids are never reused.
- `reap_reactions_for_dead_targets(group_id, dead)` payload-erases hearts
  and comments whose target group post is tombstoned or expired.
- The dead set comes from the reduced group state: expired posts plus
  tombstoned posts (a group tombstone only ever suppresses its own author's
  posts, so the deleter *is* the post author).

The drain path additionally payload-erases expired **members-only** posts
eagerly (author's and every member's stored copies), matching the
ephemerality promise for non-public profile posts: content becomes
unrecoverable at expiry, not just at window end. Public group posts are
plaintext by design and leave at bucket drop, like public profile posts.

## Consequences

- Group log count per author ≈ 1 control + live expiry windows + active
  reaction months — independent of post count.
- The genesis op shares the creator's control log (the id is allocated
  before signing and bound to `group/<GroupId>/control` after, since the
  GroupId is the genesis op's own hash).
- Old stores carry single-log groups; this lands inside the same v4 flag-day
  wipe (ADR-0016), so no migration exists or is needed.
