# 03 — The first post after a removal can seal to the old audience

**Status:** needs-triage

**Context:** lazy re-key in `JynSpaces::publish_encrypted` /
`JynGroups::encrypt_to_group`. Surfaced as the Linux CI flake in
`core/tests/circles.rs` (`circles_posts_reach_friends_of_friends_until_removed`);
the test now sidesteps it with a settle round (commit 417dceb), but the
underlying product gap remains.

## Problem

Re-keying a removed member out is lazy and deliberately best-effort inside a
publish: the pre-publish repair/reconcile "must never gag the post"
(`encrypt_to_group` logs and continues on error; the profile path is
equivalent). So the *first* post published after a removal can race its own
re-key on a slow machine and be sealed with the epoch key the removed member
still holds — they decrypt one post they were supposed to be excluded from.
CI reproduced this reliably on slow runners; it is timing, not logic, so
real devices can hit it too (e.g. publish immediately after removing
someone, on a busy phone).

One post, oldest-epoch only — but it is precisely the post a user writes
right after removing someone, which is the post they most expect that person
not to read.

## Fix direction

Make removal itself converge instead of piggybacking on the next publish:
on processing a removal (friend drop, group member removal), run the re-key
eagerly and, if it fails, queue it with retry — and have publish *block* on
a pending re-key for that context (fail closed) rather than sealing with the
stale epoch. The settle-round trick in the test documents the semantics the
product should guarantee.
