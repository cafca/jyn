# Groups follow-ups

Not a feature spec: the triage inbox for follow-ups deferred while shipping
Groups (PR #9) and porting them onto the co-deletion log model (ADR-0016 →
ADR-0018). Each ticket in `issues/` stands alone; there is no combined list.

Sources: the multi-agent code review of the Groups branch (items judged real
but out of scope for the PR), and gaps noticed while implementing the
ADR-0018 bucketing. The headline architectural item is
`issues/01-shared-sealed-context-primitive.md` — circles and group
encryption are two hand-synchronized copies of the same stack.

Delete this directory when its issues are resolved or migrated, like
`.scratch/groups` before it (a scratch area is working state, not archive).
