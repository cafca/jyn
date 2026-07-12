# A Group's Join mode and Content mode are two independent axes

**Status:** accepted

A Group carries two orthogonal properties, chosen independently — a 2×2, all
four cells legal:

- **Join mode** — how a person becomes a Member: **Open** (anyone joins, no
  approval) or **Request-to-join** (Owner approves each request).
- **Content mode** — the fixed visibility of the Group's posts: **Public**
  (plaintext, world-readable) or **Members-only** (encrypted to members).

## Cell semantics

Reading is governed by **Content mode**; posting is governed by **membership**;
Join mode governs *how* you obtain membership.

|                     | Open join                                   | Request-to-join                          |
| ------------------- | ------------------------------------------- | ---------------------------------------- |
| **Public**          | Anyone reads; membership gates posting.     | Anyone reads; Owner approves who may post.|
| **Members-only**    | Membership gates read+post; anyone self-admits to get keys. | Membership gates read+post; Owner approves who gets keys. |

## The weak-confidentiality corner is deliberate

Members-only + Open join has weak confidentiality (anyone who joins gets the
keys). We keep it on purpose: it keeps content off the plaintext firehose
without gatekeeping, and — crucially — **membership is an auditable,
append-only record.** A join is an operation in the Group's log, not a mutable
flag, so "who was able to read, and from when" is reconstructible from the
Group's own history in every mode. That visible record is a feature, not a
side effect.

## Consequences

- Reading rights derive from Content mode; posting rights derive from
  membership; the two are decoupled.
- **Every Member can post; there is no owner-only "announcement" Post mode in
  this phase.** An owner-only posting axis is a deferrable additive flag (a
  per-group check at post time) and is deliberately left out to keep the model
  at a 2×2.
- Membership is never modelled as mutable boolean state — it is a log of
  join/leave operations, so the read-eligibility timeline is always derivable.
- Because Open join exists, membership is decoupled from friendship: a Member
  need not be a friend of the Owner.
- **Roster visibility follows Content mode**: a Public group's roster and
  join/leave history are public; a Members-only group's roster is visible to
  members only. Pending join requests (request-to-join mode) are visible to the
  Owner only (plus the requester's own pending state) — a rejected request is
  never a public record.
