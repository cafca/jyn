# 02 — Expired non-public posts leave recipients' devices

**What to build:** A friend or friend-of-friend who received and viewed a non-public post has it torn down on *their* device once it expires — the media is pruned and they stop holding and serving the ciphertext — so expired content leaves the whole network, not just the author. A recipient who explicitly kept the post still keeps it, and an offline recipient catches up on next start. Behaviour is proven at the `AsyncBridge` command/event seam with multi-node (real-relay) tests, mirroring the friends-of-friends setup.

**Blocked by:** 01 — Expired non-public posts leave the author's own device (reuses the teardown path and the offline-convergence behaviour it establishes).

**Status:** ready-for-agent

- [ ] A recipient that fetched a non-public post has its plaintext cache pruned and its hold on the synced ciphertext blob dropped once the post has expired, so the recipient stops serving it and the blob becomes GC-eligible on that device.
- [ ] The recipient's decrypted-plaintext cache entry for the expired post is deleted.
- [ ] A recipient who kept the post retains their kept copy — its media survives the expiry teardown.
- [ ] If the recipient's device was offline at expiry, the teardown runs on next startup; idempotent.
- [ ] Verified two- and three-node through the `AsyncBridge` seam, deterministically — expiry driven by a past `expires_at`, no wall-clock or async-GC waits.
