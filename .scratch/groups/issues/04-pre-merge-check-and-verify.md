# 04 — Pre-merge check + final verify

**What to build:** The close-out gate. Resolve the encryption-phase-3 dependency
for the removal-clawback contract, then verify the whole feature end to end
before merging. Respect the spec's "Pre-merge checks" section and ADR-0003.

**Blocked by:** 03.

**Status:** ready-for-agent

- [ ] Determine whether encryption **phase 3 (garbage collection of expired ciphertext + blobs)** has landed on `main`.
- [ ] If phase 3 is on `main`: bring the GC-based tightening of removed-member retention (spec story 28) into scope — revise the no-clawback contract and removal semantics so removal reduces retained content via GC-driven expiry, and implement it with tests.
- [ ] If phase 3 is not on `main`: leave story 28 as the honest no-clawback contract and record the GC-based tightening as a follow-up (not part of this merge), with a note pointing at the phase-3 dependency.
- [ ] The full `AsyncBridge` integration suite (public + members-only Groups, join/governance, discovery, hearts) passes over a real relay; the Flutter provider-level tests pass.
- [ ] The alignment rules in ADR-0014 hold in the shipped code (role-as-set, append-only membership, extensible governance op set, nothing anchored to the creator, Group as a first-class actor, composable identity/topic).
