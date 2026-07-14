# 07 — Fixed sleeps run while holding the sync mutex

**Status:** needs-triage

**Context:** review finding deferred from PR #9. `core/src/sync.rs`
(`CONTACT_BOOTSTRAP_SETTLE_MILLIS` sleeps at ~448, ~511, ~646 and the retry
loop at ~523).

## Problem

Several bootstrap/settle paths `tokio::time::sleep` while the caller holds
`state.sync` (the tokio mutex every bridge command serializes on). A 750 ms
settle wait blocks *every* other command — post publishes, UI reads — for
its full duration; the retry loop can hold it for seconds. Correctness is
unaffected; responsiveness is.

## Fix direction

Restructure so waits happen with the lock released (do the read, drop the
guard, sleep, re-acquire), or move the settling work off the command path
into the maintenance task, which already owns retry pacing.
