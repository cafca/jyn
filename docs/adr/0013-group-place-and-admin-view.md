# Group place screen, with administration as a dedicated sub-view

**Status:** accepted

## Group place

A single Group's screen, profile-like, adapting to viewer state:

- **Header** — name, member count, Content/Join/Discoverability indicators, and
  the viewer's status (owner / member / non-member).
- **Group stream** — the group's posts, reverse-chron, same post cards as the
  river.
- **In-group composer** (members only) — posts go to this group only; no
  visibility dial (fixed by Content mode); lifetime per-post.
- **Affordances by viewer state:**
  - Non-member, public group — reads the stream; sees **Join** (open) or
    **Request** (request-to-join).
  - Non-member, members-only group — sees identity + Join/Request, no content
    until admitted.
  - Member — read + compose + **Leave**; roster visible per Content mode
    ([ADR-0002](0002-group-join-and-content-modes-independent.md)).

## Administration is a dedicated view

Owner governance is **not** embedded inline on the place screen; it lives in a
dedicated **Group admin** view, reached from the place (an affordance visible
only to the Owner). It hosts: edit name / Join mode / Discoverability, approve
or deny **pending requests**, **remove** members, and **transfer ownership**.

(Share codes / invite links are **not** in scope for this phase — see the spec's
Out of Scope. They may be added later as an out-of-band join path.)

This is a deliberate exception to the app's usual "settings live inline where
they govern" pattern — group administration is a distinct enough surface (and,
looking ahead to multi-admin, a shared governance space) to warrant its own
view.
