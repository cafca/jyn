# Spec: End-to-end encryption for non-public posts

Agreed 2026-07-05. Adds p2panda group encryption so that `Friends` and
`Circles` posts (and their media and comments) replicate as ciphertext
instead of plaintext. `Public` posts stay in the clear; `Private` stays
local-only. Built on `p2panda-spaces` (`p2panda-auth` + `p2panda-encryption`).

Today `Friends` posts replicate in **plaintext** — `Visibility` is enforced
only at read time. This spec makes non-public content confidential on the
wire and at rest.

## Encryption model

- **Data encryption, not message encryption.** Use the `p2panda-encryption`
  *data-encryption* scheme (long-lived symmetric group secrets), via
  `p2panda-spaces`. Posts are durable, replicated objects that late-joining
  peers must be able to read — the forward-secure *message* scheme fights
  that model.
- **Ephemerality is a retention property, not a key-schedule property.**
  "New friends can't see my expired posts" is delivered by **garbage-
  collecting expired ciphertext + blobs** (Q1/Q3), *not* by forward secrecy.
  The data-encryption scheme gives new members past content by default; it
  won't fight GC, but it won't do that job either.
- *Rejected:* message encryption (Option B). It would destroy your own
  history on reinstall/new device, thrash on membership churn, and fight
  out-of-order sync. Reserved for a possible future **DM** feature, where
  ordered delivery and true forward secrecy actually fit. Note `keep_post`
  already lets any friend retain plaintext, so crypto-ephemerality against
  your own friends was never real anyway.

## Groups

- **Sender-defined, single-admin audience groups (spaces).** Every user owns
  their own `Friends` space and `Circles` space and is the **sole admin**.
  You encrypt a non-public post to one of *your* spaces; your friends
  encrypt to *theirs*. `Friends` and `Circles` are the **same primitive** —
  a single-admin group with an explicit member list — differing only in how
  the member list is populated.
- **Admin model via `p2panda-auth` roles.** Access levels are
  Pull / Read / Write / **Manage**; only Manage members can change group
  state. You are the only Manage member of your spaces — this *is* the
  "a group only I administer" requirement, delivered by the crate.
- *Rejected:* shared/mutual multi-admin groups (contradicts "only I
  administer", causes epoch forks); pure pairwise wrapping (kept only as a
  theoretical fallback if the crate proved unusable — it didn't).

## Circles membership (friends-of-friends)

- **Fully auto-derived.** `Circle = ⋃(each friend's friends)`, recomputed as
  friends' friend lists change.
- **Friend lists are mandatorily visible to your friends** (whole list, no
  opt-out) — this is what makes FoF derivation reliable. Accepted cost:
  your social graph is visible to your friends and seeds their Circles.
- **Re-key churn is mitigated by lazy re-keying.** In data encryption,
  *adding* a member is cheap; *removing* one forces a group re-key. FoF
  shrinks constantly, so re-key **lazily — right before the next Circle
  post** — bounding re-key frequency to publish frequency, not churn
  frequency.
- *Rejected:* manual curation (throws away FoF); suggest-and-approve hybrid
  (chosen against in favor of full automation).

## What is encrypted / what leaks

- **Encrypt the payload; blind the audience.** Post body + media keys are
  encrypted. The operation *type* moves inside the ciphertext and the
  cleartext key id is an **opaque random handle** (not "friends"/"circles"),
  so a passive peer sees only "an encrypted op from Alice at time T" — not
  the audience tier or whether it's a post, like, or comment. Recipients
  trial-decrypt against their known epoch keys.
- **Unavoidably in the clear** (p2panda header — sync/ordering needs it):
  author `profile_id`, sequence number, timestamp, backlinks, payload
  hash/size.
- *Rejected:* minimal metadata (Option A — leaks tier + op-type);
  metadata-private with padding/cover traffic (Option C — too heavy, hurts
  sync).

## Key delivery

- **`p2panda-spaces` key bundles.** Each user publishes an X25519 key bundle
  (long-term key + one-time prekeys) under their identity; the crate
  establishes pairwise channels, distributes/rotates group secrets, and
  handles removal re-keys. Bundles **auto-rotate near expiry**.
- The single-admin model is *not* a mismatch here: `p2panda-auth`'s
  Manage-only permission is exactly the admin control we want, and we get
  post-compromise security on the key agreement for free.
- **Accepted costs** (beyond tracking upstream releases): group/auth-CRDT
  state is **opaque and not re-derivable from the identity key alone** (see
  Recovery), and true **multi-device depends on an in-flight upstream
  feature** (see below).

## Media / blobs

- **Per-blob random symmetric key, wrapped inside the encrypted payload.**
  Each attachment is encrypted with its own fresh key; the key rides in the
  group-encrypted post payload. The blob replicates as **ciphertext**; every
  member fetches the one shared ciphertext and decrypts locally. Decoupled
  from group re-keying — removing a member never re-encrypts media.
- **Dedup without convergent encryption:**
  - *Fan-out* (one post → many recipients) dedups for free — one ciphertext,
    one content address, shared by all recipients.
  - *Reshares* reference the **original** ciphertext blob and re-wrap its
    existing per-blob key to the new audience — same bytes on the network,
    stored once.
- *Rejected:* convergent encryption (`key = hash(plaintext)`). It's only as
  strong as the plaintext's entropy (confirmation/dictionary attack on
  known media), buys no key-management saving (the key still ships in the
  payload), and reintroduces the equality-metadata leak Q5 removed. Keyed
  convergent encryption was left on the table only for a hypothetical
  high-volume independent-identical-upload case, which we don't have.

## Comments and reactions

- **Encrypted to the same space as the post; whole audience sees the
  thread.** Members hold the space key (Read) and can contribute
  comments/reactions the audience can read (Write); you keep Manage.
- **Stated explicitly:** a shared group key means *any Write member can
  encrypt content the whole group will decrypt*. The app constrains
  *meaning* via signatures + operation types (a comment must attach to a
  post); the key doesn't. This is inherent to shared-key groups and
  accepted.
- *Rejected:* author-only reactions (degrades to a broadcast feed).

## Recovery and multi-device

- **Identity key is the root and must be backed up** — seed phrase + OS
  secure keychain. Losing it is catastrophic regardless (new identity, lost
  friendships).
- **Back up the opaque group state too.** Periodic snapshots of the space /
  auth-CRDT state, **encrypted to the identity key**, written to storage the
  user controls (local export + **auto-sync to the user's own iCloud/Files
  by default** — never a server we run). Restore = import identity +
  snapshot → deterministic full recovery, no dependence on peers.
- **Blob backup — full for live + kept, never for expired (Option C).**
  Default includes full blob ciphertext for currently-live and `keep_post`
  media; **expired blobs are never backed up** (else ephemerality is a lie).
  Blobs are already ciphertext, so cloud storage learns nothing. A setting
  offers **full / kept-only / metadata-only** for tight quotas;
  ephemerality holds in every mode.
- **Multi-device is deferred.** Upstream multi-device is in-flight; two live
  devices on one identity would fork the auth CRDT. Enforce **one active
  device**; "new device" is a *migration* (restore from backup). Revisit
  when upstream lands.
- *Rejected:* identity-key-only backup (Option A) — silently risks losing
  access to your *own* history when peers have GC'd key-agreement messages.

## Migration

- **None. Flag-day wipe.** Introducing encrypted operations is a breaking
  protocol change; ship it with a version bump and **wipe all content, logs,
  blobs, and social-graph operations**. Re-encrypting already-plaintext
  posts (they've already replicated in the clear) would be security theater.
- **Keep the identity keypair.** `node.key` / `profile_id` / friend code
  stay stable; the new X25519 bundle publishes under the same identity.
  Users re-add friends, but shared friend codes still resolve.

## Rollout

- **Phase 1 — Encrypted Friends, safely recoverable (the flag-day
  release).** X25519 key bundles → `p2panda-spaces` integration → single-
  admin Friends space → encrypted payloads (opaque key id, blinded
  tier/op-type) → per-blob media encryption → comments/reactions in-space →
  **identity backup + crypto-state snapshot**. The wipe ships here.
  Recovery is in Phase 1 by rule: the moment users create encrypted posts,
  that data is irreplaceable, so a lost device must not mean total loss.
- **Phase 2 — Circles + full backup.** FoF derivation (mandatory friend-list
  visibility → membership → lazy re-key on removal) + Option-C blob backup
  with the full/kept/metadata toggle.
- **Phase 3 — Hardening & optimization.** Ephemerality GC of expired
  ciphertext/blobs; reshare-by-reference dedup.
- **Later — Multi-device**, when the upstream feature matures.

## Open risks

- **Upstream maturity.** `p2panda-spaces`/`p2panda-auth`/`p2panda-encryption`
  are young and moving; the unified integration layer and multi-device are
  still under active development. Pin versions; budget for API churn.
- **Opaque state backup is load-bearing** — corruption or loss can lock a
  user out of their own groups. Snapshot integrity (and restore testing) is
  a first-class concern, not an afterthought.
- **Friend-list exposure is mandatory** — a deliberate privacy stance worth
  surfacing clearly to users at onboarding.

## Phase 2 implementation notes (2026-07-11)

- Circles is a second single-admin space per profile (id derived via HKDF
  `jyn/circles-space/v1`). Which of an owner's two spaces is which stays
  blinded on the wire; members learn the mapping from the visibility of
  posts they could decrypt, and interactions route by it.
- Two spaces per author exposed an upstream flaw: auth operations chained
  on *global* graph heads, so one author's groups referenced each other and
  space copies (which only witness their own group) held dangling graph
  references — the auth resolver panicked once concurrency touched them.
  Fixed on the fork branch `circles-group-scoped-auth` (p2panda-spaces:
  group-scoped dependencies, repair and repair-detection). Trade-off:
  nested groups as space members don't resolve on that branch; jyn only
  adds individuals.
- Circle members' (friends-of-friends) topics are synced so their key
  bundles and their circles posts flow; the river shows their posts. The
  automatic follow-back is guarded by the outgoing-requests list so a
  synced non-friend can't befriend us by following us.
- Backup media modes: full (default) / kept-only / metadata-only. Expired
  blobs are never archived; restore stages blob bytes and the next start
  re-imports and re-pins them from the restored records.
