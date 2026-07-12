# Group identity, metadata, and creator-independence

**Status:** accepted

## Identity

A **GroupId** is the hash of the group-creation (genesis) op on the creator's
log — a stable, unforgeable, owner-bound identifier. Ownership *transfer* moves
the `Manage` role; the GroupId never changes.

## Metadata and mutability

A Group carries: display **name**, **Content mode** (public / members-only),
**Join mode** (open / request-to-join). Stored as the genesis payload plus
later metadata-update ops by the `Manage` holder.

- **Name** — mutable by the Owner anytime.
- **Join mode** — mutable by the Owner anytime (it is just the auto-accept vs
  manual-accept policy of [ADR-0005](0005-join-is-owner-node-policy.md); no
  retroactive data hazard).
- **Content mode** — **immutable after creation in this phase.** Flipping it is
  hazardous: public→members-only cannot retroactively encrypt already-published
  plaintext, and members-only→public would expose previously-encrypted content
  and the roster. Mode migration is deferred and, if ever built, is an explicit
  "migrate to a new group" operation, not an in-place flip.

## Creator-independence (forward-looking constraint)

The GroupId may be *derived* from the creator's genesis op, but the group's
**live state and authority must be fully portable off the creator.** The
creator must be able to transfer `Manage` to an entirely different user *and
then leave the group completely*, with the group persisting and depending on
the creator's node, identity, or data for nothing beyond the historical genesis
op that minted the id.

**Data-model consequence:** never anchor mutable group state (membership,
metadata, seeding) to the creator specifically. Anchor it to the *current*
`Manage` holder and to the member set. The genesis op's only permanent role is
minting the id; the group must survive the creator's total departure.
