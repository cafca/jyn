# Screen Design Foundations: Mental Models and Affordances (v1)

**Date:** 2026-07-04
**Status:** Design-input document — the outcome of the second user-research interview. Defines the mental models, objects, and affordances the first set of screen designs must express. Deliberately silent on visual styling.
**Builds on:** `2026-07-03-product-direction.md` (the wound, poster sovereignty, trusted-taste heartbeat, intimate-to-community trajectory).

## The metaphor system: water

The app speaks water throughout.

- **The river** — home; the flowing, interleaved surface where everything passes by.
- **The pond** — a person's accumulation of permanent posts; stillness, what remains.
- Further vocabulary (whatever groups, kept things, and hearts end up called) is **open** — to be named by the owner within the water theme, not invented by designers.

The metaphor is load-bearing, not decorative: flow vs. accumulation *is* the ephemeral/permanent mental model.

## Core mental model

### One post type, one differentiating property

There is exactly one kind of post. Its defining property is **lifetime**: finite (ephemeral) or permanent. There are no separate "stories," no "shelf items," no announcement cards. Visual differentiation in the river expresses lifetime and nothing else. **Readers always see a post's remaining lifetime** — ephemerality is an honest, visible contract, never a surprise.

### Context is exclusive and chosen by place

A post lives in exactly one context, determined by **where it was created**: composing in the river posts to your personal stream; composing in a group (post-v1) posts to that group only. No cross-posting, no destination dropdown. It must always be clear what the context of a post is.

### Visibility is the author's dial

Per-post visibility, chosen by the author: **public / circles (friends-of-friends) / friends only / private**. Defaults for visibility and lifetime are set per profile, pre-filled in the composer, adjustable in place. *Private* posts make the app a usable solo journal from minute one. Groups (post-v1) carry a fixed visibility of their own instead of per-post choice.

### Sovereignty mechanics

- **Edit:** authors can edit published posts; the post shows an "edited" mark.
- **Delete:** the author's delete reaches everywhere — including into readers' kept copies.
- **Promote:** the author may change an ephemeral post's lifetime to permanent, at which point it joins their pond.

### Propagation: hearts, not reposts

There are **no reposts**. The only propagation is the **like (heart)** — and likes are **named**, never anonymous counts. Liking a post surfaces it on your friends' rivers *if the author's visibility permits* (circles/public). The post never leaves its context, is never copied, never loses its author; a heart only routes attention. This is the entire taste economy: "found something cool through people I trust" is literally a post arriving framed "♥ Bob."

### Keeping is a lease

You can keep things in a private collection, but the kept copy is subordinate to the author: it expires when an ephemeral post's lifetime ends, and it dies when the author deletes — for ephemeral and permanent posts alike. Receiver attachment never overrides author intent.

### Relationships: one edge, consented, two doors

v1 has exactly one relationship: **consented friendship**. No unilateral follow, no subscriber asymmetry.

- **Share-code ritual** — the out-of-band introduction, for connecting with someone who can't see you yet (uses the existing plumbing).
- **In-app friendship request** — from a profile that discovery has already put in front of you; they accept or decline.

### Deliberate deviations from Patchwork/SSB

The owner knows SSB/Patchwork well and deviates on purpose: explicit group containers will exist alongside ambient personal streams (post-v1); per-post visibility levels exist (SSB had none); edits are allowed (SSB forbade them); edges are consented (SSB follows were unilateral and public).

## The v1 screen set

Four screens. Groups are **entirely absent** from v1 — no group places and no group-digest doors in the river.

### 1. Home river

One scrolling surface, **reverse-chronological** at launch (a ranked view exists in the model but is deferred — including the question of whose algorithm ranks).

Contents:
- **Posts** from friends — one unit type; ephemeral posts show remaining lifetime, permanent posts read as lasting.
- **Discovery cards** — posts by non-friends carried in by a friend's named heart, shown only when author visibility (circles/public) permits, framed with provenance ("♥ Bob").
- **Inline composer** — posting here = your stream; profile defaults for visibility and lifetime pre-filled.
- **Comment previews** — a line or two of thread under each post.
- Per-post affordances: author, context badge, inline-consumable content (audio plays, video plays, text reads in place), named hearts, keep, comments, lifetime.

The ambient machinery layer (Bevy-animated sync/peering/ephemerality decoration) is **postponed** — see Deferred.

### 2. Profile

*As simple as possible in v1.*

Anyone's profile:
- **Identity surface** — name, image, self-description; the identity a person is building by posting.
- **One stream** — all their posts visible to you, every lifetime mixed. There is no separate pond section: ephemeral posts drain away on expiry, so the profile stream *becomes* the pond over time. Accumulation by survival.
- **Friendship affordance** (on a stranger's profile) — request friendship; no follow button exists.

Your own profile, additionally:
- **Promote** on your own posts.
- **Defaults** — your default visibility and lifetime are set here (settings live where they govern).
- **Friends** — your friends list, pending friendship requests, and share-code generation/entry all live here, not on a separate screen.

### 3. Post view

- **The artifact** at full attention.
- **Its situation:** author, context, visibility level, lifetime (remaining time or permanence).
- **Provenance framing** when a heart brought you here ("you can see this because Bob ♥ it").
- **The full comment thread** and its composer.
- **Reader affordances:** named heart, keep (with its lease semantics).
- **Author affordances:** edit (marked), delete (reaches kept copies), promote.

### 4. Onboarding and the first hour

- **Identity creation** — a keypair born on the machine; the user gives it a name and a face. No account, no email, no server.
- **No key backup in v1** — consciously accepted: a dead disk is a dead identity for now.
- **The share-code ritual** and **pending-request handling**.
- **The empty river** — because private posts exist, the app is fully usable alone from the first minute (journal, private pond). The empty state invites *being*, not just inviting.

## Settings principle

There is **no engine-room screen and no monolithic settings page**. Settings are split into a small global layer plus **context-specific settings embedded in the views they govern** (profile defaults on the profile, post controls on the post). The under-the-hood machinery view is not a separate destination; legibility is delivered in context.

## Design-language direction (recorded, not yet designed)

The owner intends a **legible commons**: the p2p machinery (sync progress, peering, seeding capacity, ephemerality) eventually surfaces as **animated, decorative, living elements built in Bevy** — the infrastructure as the weather of the app, with a literal look-under-the-hood available on demand. This is postponed for the first screen set but should inform structural choices (the water world and the machinery-as-ambient-life idea are made for each other). Aesthetics beyond this note are out of scope for this document.

## Deferred (post-v1 inventory)

- **Groups** — hard containers, fixed visibility, their own composers; group-digest doors in the river; group place screen; governance inside groups.
- **Ranked river view** — and the sovereignty question of whose ranking.
- **Key backup / multi-device identity.**
- **Unilateral follows** — possibly never; revisit when discovery outgrows friendship.
- **Ambient machinery layer** — the Bevy living-decoration system.
- **Mobile** — light clients on friend-group-owned nodes, per the direction doc.

## What the screen designs must prove

A first set of screens succeeds if someone looking at them can correctly infer the mental models without being told:

1. Ephemeral and permanent things share one river, and lifetime is always honest.
2. A post's context is unmistakable.
3. Taste travels by named hearts, never by copies.
4. The author owns the post — everywhere, forever, including your kept copy.
5. Friendship is consented, and there is nothing else.
6. Alone, the app is still a place to be.
