# 02 — Group ciphertext on non-member devices is never GC'd

**Status:** needs-triage

**Context:** ADR-0018 bucketing (`JynOperationDomain::drop_drained_buckets`,
`core/src/domain.rs`). Noticed while writing the ADR — its first draft
overclaimed here and was corrected in the same commit as this ticket.

## Problem

`drop_drained_buckets` decides drainability by reading payloads
(`effective_operation`). An undecryptable `Spaces` wrapper classifies as
`Opaque`, which deliberately keeps its log alive — GC must not drop content
it cannot read. Correct for members (their decrypted cache rows make
wrappers `Decoded`, so expired posts classify dead and buckets drain), but a
**non-member** holding a members-only group's ciphertext can never classify
anything: every wrapper is `Opaque`, so no bucket on that topic ever drains
on their device.

Non-members do hold such ciphertext: a heart discovery card calls
`syncGroup` before the join answer arrives (`home_screen.dart`), and a
denied or never-answered requester keeps whatever the topic already synced.
The result is unbounded, undeletable ciphertext retention on devices that
were never welcomed — the opposite of the co-deletion promise, and the
expired content also keeps being *served* to peers from there.

The same `Opaque`-keeps-alive rule applies on profile topics (a
friend-of-friend who lost circle access), so a fix generalizes.

## Fix direction

The bucket window is exactly the payload-free drop criterion (ADR-0016), but
only the author's registry knows a log's window. Options: (a) author-side,
prune own expired buckets by registry context so peers' catch-up sync
converges on the pruned log (recipients still hold stale copies until they
notice the author's head); (b) let a recipient drop `Opaque`-only logs after
a generous fixed retention (says "ciphertext I could not read for N months
leaves"); (c) carry a coarse, blinded retention bound in the header so any
holder can window-drop without learning the audience. (c) changes the wire
format; (b) is local-only and cheap.
