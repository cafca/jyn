# Product Direction: From File Sharing to Trusted-Taste Social

**Date:** 2026-07-03
**Status:** Direction document — the outcome of a user-research interview with the project owner. Not an implementation spec.
**Scope:** The app currently living in `file-sharing/` on the `port-blobs-to-net-v0.5` branch, built on the p2panda fork in this repository.

## Origin: the wound

Shared things kept dying because someone other than the people sharing them decided they should. The same injury at three scales:

- A Reddit community had threads removed and moderators replaced by decisions made outside the community.
- GeoCities — an entire ecosystem of published creative work — was shut down by its owner.
- A YouTube video sent to a friend was a dead link two years later, when the friend wanted to watch it again.

The pain was never moderation, storage, or file transfer. It was **authors losing authority to platforms**. Peer-to-peer is the means, not the end: when the people who care about something hold the copies, there is no outside position from which to erase it.

## The principle

> **The poster is sovereign, not the platform.**

Every hard call so far falls out of this sentence:

- **Lifetimes.** An author may give content a short lifetime, and expiry removes it from receivers' devices too. A receiver's attachment does not override the author's intent. Deletion by the author is legitimate; erasure by an institution is the enemy.
- **Moderation.** Deferred, on principle. At intimate scale there are only authors — no institution exists to impose values. Governance machinery becomes a real question only when communities of strangers arrive (see Open Questions).
- **The builder's role.** The project owner does not become the new gatekeeper. No identity, data, or infrastructure routes through anything the builder operates.

## The product

Not a file browser. The current app's folder-sharing UX was an exploration of the plumbing; the product needs a clean-slate user experience.

The product is a **social stream among people who trust each other's taste**. Three content types flow through it:

1. **Fragments recorded in the moment** — voice notes, quick videos. Spontaneous, low-stakes, typically short-lived.
2. **Finished works** — music, videos, writing. Deliberate and durable.
3. **Found inspiration** — memes, videos, things curated from elsewhere. The found thing itself is held by the group, not linked, so it can never rot the way that YouTube link did.

**The heartbeat** — the reason a person opens the app on an ordinary Wednesday — is the third one:

> *"I found something cool through people I trust."*

Discovery through trusted taste. Fragments and finished works ride along, but curation is the core rhythm. Content types carry **different default lifetimes**, always set by the author.

## The trajectory

**Start intimate, grow toward community.**

- **Stage 1 — a handful of close friends.** Three to five real people. The existing contacts-via-share-codes model already serves this scale. Moderation needs are near zero: among friends, "moderation" is unfollowing someone or telling them off.
- **Stage 2 — interest communities.** Dozens to hundreds of partial strangers gathered around shared taste, governing their own space. This is the destination, not the beginning.

Taste is the right vehicle for this journey: taste networks scale from friends to strangers in a way intimacy never does. An app whose heartbeat is "feel close to my friends" caps out at friendship; an app whose heartbeat is trusted discovery grows naturally into communities of interest.

## The architecture stance

**An asymmetric network: desktop backbone, mobile antenna.**

- **Desktop devices are full, always-on nodes.** They hold storage and make permanence real. Desktop-first is not a compromise — it is building the backbone before the antenna. (Later: small home servers can play the same role.)
- **Mobile is existential, but arrives second.** Fragments are recorded on phones; found things are consumed on phones in spare minutes. Mobile devices connect as light clients to always-on nodes **owned by the friend group itself** — so sovereignty holds because the infrastructure belongs to the people, not to a company.

## The ambition

The two-year happy ending: **a small ecosystem of communities running on it that the builder does not control.** Not a job, not a personal tool — a thing that escaped.

This quietly mandates:

- Open source, forkable, self-hostable.
- No central identity service or data path through the builder's infrastructure.
- Eventually, protocol stability valued over feature velocity — other people's communities must not break when the builder changes their mind.

## Implications for the current codebase

- The p2panda plumbing work (blobs over net, sync, contacts, share codes, pause/resume, recovery) retains its value: it is the substrate the new product needs.
- The folder-sharing UX does not carry forward. The next product iteration is a stream/feed experience designed around posting and discovering, not browsing directories.
- The contact model (follow via share codes) maps directly onto Stage 1 and survives.

## Resolved tensions

| Tension | Resolution |
|---|---|
| Permanence vs. ephemerality | Author-chosen lifetimes per content type; receiver attachment does not override author intent |
| Intimate vs. community scale | Both, in sequence: friends first, communities as destination |
| Desktop vs. mobile | Asymmetric network: desktop backbone first, mobile light clients second |
| Who may delete | The author always; nobody else ever |

## Open questions (deliberately deferred)

1. **Found content and copyright.** Archiving an actual video among five friends is private sharing; the same behavior in a public interest community is classic p2p legal territory. Somewhere on the intimate-to-community road, this needs a stance.
2. **What expiry technically means.** In a p2p system, cooperative deletion is a protocol promise, not a physical guarantee. Acceptable among friends; matters more among strangers.
3. **Community governance.** When Stage 2 arrives: who is "the author" of a shared space? What does poster-sovereignty mean for a community artifact?
4. **Mobile gatekeepers.** App stores are themselves value-imposing platforms. The sovereignty story on iOS in particular needs care (distribution, background execution, payment rules).

## What's next

1. Sketch the overhauled UX around the heartbeat: what does the stream look like, what does posting each of the three content types feel like, how do lifetimes appear in the interface?
2. Define the first thin slice toward Stage 1: the smallest version that three real friends would actually use in a week.
3. Revisit the open questions when — and not before — the stage that makes them real approaches.
