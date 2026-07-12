# Hearts on group posts: outward discovery iff Public AND listed

**Status:** accepted

Hearts are the app's only propagation (a like can surface a post on the liker's
friends' rivers if author visibility permits). For group posts the rules fall
out of Content mode + Discoverability:

- **Members-only group** — a heart is an **in-group named like only**; it never
  produces a discovery card to non-members. Confidentiality wins
  unconditionally (a non-member must never be pointed at content they cannot
  read).
- **Public + `listed` group** — hearts behave like a normal public post's
  heart: a named discovery card on the liker's friends' rivers, framed with
  provenance and **pointing into the group context** ("♥ Bob, in *Group X*").
  The post is not copied or moved; the card is a pointer into the group place,
  preserving context exclusivity.
- **Public + `unlisted` group** — hearts stay **in-group**; no outward
  discovery card.

**Unifying rule:** outward heart-discovery happens **iff Content mode = Public
AND Discoverability = `listed`.** Otherwise a heart is a purely in-group named
like. This makes `unlisted` mean exactly one thing — no automatic outward
surfacing of the group by any mechanism (neither membership advertisement nor
heart-driven discovery).
