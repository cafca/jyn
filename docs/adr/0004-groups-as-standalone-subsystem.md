# Groups is a standalone subsystem; single-admin spaces become a special case later

**Status:** accepted

Groups is built as its own subsystem (`core/src/groups/`) with its own model for
group identity, membership, join protocol, and posting. It uses p2panda
primitives (`p2panda-auth`, `p2panda-encryption`) directly and does **not** build
on, extend, or entangle with the existing per-profile `JynSpaces` module.

## Why not reuse / extract

We considered (a) generalizing `JynSpaces` in place and (c) extracting a shared
single-admin-group core that both `JynSpaces` and Groups consume as peers. Both
were rejected. The reason is directional: **we anticipate single-admin spaces
(Friends/Circles) becoming a *special case of Groups*, not a fundamental
building block.** A shared-core extraction (c) would freeze the two as
permanent peers; generalizing in place (a) would overload `JynSpaces` with
responsibilities it will eventually shed. Building Groups as the more general
primitive now is the shape that the future migration wants.

## Consequences

- Accept some near-term duplication of crypto plumbing (auth group, re-key,
  seal/open payload) between `JynSpaces` and `groups`. This is temporary: the
  intended endgame is that Friends/Circles are re-expressed as (auto-derived,
  blinded, always-encrypted) Groups, and the bespoke `JynSpaces` module retires.
- `JynSpaces` is left untouched by this phase — no refactor, no risk to the
  existing Friends/Circles behavior and its integration tests.
- The Groups subsystem should be designed so that a per-profile, auto-derived,
  always-encrypted, blinded Group is expressible in its model — that is the
  litmus test for "spaces is a special case of groups."
