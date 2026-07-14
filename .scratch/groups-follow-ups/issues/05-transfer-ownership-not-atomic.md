# 05 — Ownership transfer is two ops; a crash between them strands two Manage holders

**Status:** needs-triage

**Context:** `JynGroups::transfer_ownership` (`core/src/groups/service.rs`).
Review finding judged real but deferred from PR #9 (recoverable state, no
data loss).

## Problem

Transfer is promote-then-demote: a `GroupGoverned` op granting the new Owner
`Manage`, then one revoking the old Owner's. A crash (or permanent failure)
between the two leaves the group with two `Manage` holders. Reduction is
consistent — both really do hold Manage — but the phase-one single-owner
invariant (ADR-0003/0014) is silently broken: either can govern, remove the
other, or transfer again, and UI that assumes exactly one Owner
(`ReducedGroupState::owner` returns the first match) shows whichever sorts
first.

Recoverable by hand (the stale Owner can be demoted with another governance
op), but nothing detects or repairs it automatically.

## Fix direction

Either make the transfer one atomic governance action (a single
`SetMemberRoles`-shaped op that swaps both role sets — wire-visible change),
or add a reducer/owner-duty repair rule: on seeing two Manage holders, the
*later*-promoted one wins and the earlier is treated as demoted (or the
owner's node emits the missing demote on its next duty pass).
