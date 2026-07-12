# Groups launch single-owner, with a migration path to shared multi-admin

**Status:** accepted

A user-facing Group is a hard container people post into. The design calls for
"governance inside groups", which ultimately means shared, multi-admin
governance. But the encryption layer today is single-admin per-user spaces
(`p2panda-spaces`), and shared/multi-admin groups were explicitly rejected in
the post-encryption spec because they reopen the epoch-fork problem and need a
new `p2panda-auth` mode.

We will ship Groups as **single-owner** first: one member holds the sole
`Manage` role and governs membership; everyone else posts and reads. This
reuses the mature single-admin space primitive, sidesteps epoch forks, and is
tractable for autonomous agent implementation.

Shared multi-admin governance is a **committed fast-follow, not a maybe.** The
data model must therefore treat ownership as a *role held by a member*, not a
hard-coded singleton or a property of the group record — so that promoting a
second member to `Manage` later is an additive change, not a schema/crypto
rewrite. Any design choice that would make the A→B transition expensive is
disqualified even if it is simpler for A alone.

## Considered Options

- **(A) Single-owner group** — chosen. Sole `Manage` admin; reuses existing
  single-admin space verbatim; no new crypto.
- **(B) Shared multi-admin group** — the eventual target, deferred. Reopens
  epoch-fork handling and needs a new auth-CRDT mode; too much unsettled
  research for the first cut.

## Consequences

- Ownership is modelled as an assignable role from day one, even though only
  one member can hold it in A.
- Downstream decisions (membership representation, role storage, encryption
  mapping) are all constrained by "must not make A→B expensive."
