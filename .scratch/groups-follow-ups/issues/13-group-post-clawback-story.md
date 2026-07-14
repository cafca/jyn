# 13 — Spec story 28: clawback after removing a member

**Status:** needs-triage

**Context:** Groups spec story deliberately deferred from PR #9 (product
scope). Related mechanism: recipient-side teardown (`drain_expired`,
`core/src/bridge.rs`) erases *expired* members-only content on every
member's device.

## Problem

Removing a member re-keys future posts away from them (modulo issue 03),
but everything they already synced and decrypted stays readable on their
device forever. The spec's clawback story wants removal to also reach the
past: the removed member's device should tear down the group's decrypted
content the way expiry teardown does.

Honest limit: clawback is cooperative — a compliant client erases; a
modified one keeps what it saw. The story is about the honest-client
default, not a cryptographic guarantee.

## Fix direction

On processing one's own removal (the DCGKA remove reaches the removed
member), run the group teardown locally: delete decrypted cache rows,
payload-erase wrappers, prune media cache, stop serving the group's blobs —
the same primitives `drain_expired` already uses for expired members-only
posts (ADR-0018).
