# 01 — Expired non-public posts leave the author's own device

**What to build:** When my own Friends or Circles post expires — or when I delete it — everything readable about it is removed from my device: its photos/videos are torn down and the locally-decrypted copy of its text is purged. Posts I kept survive; permanent and public posts are untouched; and if my device was offline at the moment of expiry, the cleanup happens the next time it starts. This is the core teardown path, built once and reused by both expiry and delete. Behaviour is proven at the `AsyncBridge` command/event seam and, for the plaintext-cache purge, at the operation-domain unit seam.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] When an author's own Friends/Circles post has expired and the expiry drain runs, its attachment pins are removed and its materialized plaintext cache files are pruned, so the blobs lose their GC root.
- [ ] The decrypted-plaintext cache entry for that post is deleted, so its readable text cannot be recovered from local storage even though the encrypted operation record remains.
- [ ] Deleting a non-public post reaches the same end state (decrypted plaintext purged), so delete and expiry converge on one teardown.
- [ ] A post the author kept survives its own expiry: the kept copy's media stays held via its own pin namespace, and only the feed presence is torn down (pin-counting — a shared blob is reclaimed only when the last referencing pin is gone).
- [ ] Permanent (never-expiring) posts and public posts are left completely untouched by teardown.
- [ ] If the device was offline at expiry, teardown runs on next startup; re-running teardown on an already-torn-down post is a no-op.
- [ ] No wall-clock waits and no waiting on the store's async GC: expiry is driven by an `expires_at` set in the past, and teardown is asserted on synchronously-controlled state (pins, cache files, decrypted row); if physical byte absence is ever asserted it goes through a mock/seam over the blob store, not a GC timer.
