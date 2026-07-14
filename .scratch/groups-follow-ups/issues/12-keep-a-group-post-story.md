# 12 — Spec story 23: keeping a group post

**Status:** needs-triage

**Context:** Groups spec story deliberately deferred from PR #9 (product
scope). Profile posts have keeps (`state.keeps`, lease semantics in
`drain_expired`); group posts have none.

## Problem

A member cannot keep a group post: there is no keep flow for group content,
so an expiring group post leaves everyone's device with no lease mechanism —
and with ADR-0018 its bucket now actually drops, so "it stayed in my store
anyway" no longer papers over the gap.

## Fix direction

Decide the product rule first: are keeps of members-only content allowed at
all (an ex-member's keep outliving their membership is a real leak vector —
compare the clawback story, issue 13)? If yes, mirror the profile keep:
snapshot + `keep/…` blob pins + lease enforcement in `drain_expired`, plus
the ADR-0018 GC exemption keeps already get via their own pins.
