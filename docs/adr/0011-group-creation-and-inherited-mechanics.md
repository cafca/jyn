# Group creation policy and inherited post mechanics

**Status:** accepted

## Creation

Any user can create a Group and becomes its Owner — no gating, no cap. Creation
mints the GroupId (genesis op on the creator's log) and sets **name, Content
mode, Join mode, Discoverability** at once. Content mode is frozen thereafter
([ADR-0006](0006-group-identity-and-metadata.md)); name, Join mode, and
Discoverability stay Owner-editable.

## Inherited mechanics (derived, not new choices)

- **Comments** on a group post inherit the post's Content mode: members-only →
  encrypted to the group's space (same path as members-only post comments
  today); public → plaintext. Threading unchanged.
- **Keep (lease)** works with the existing lease semantics — kept copy
  subordinate to the author, dies on delete/expiry. A member who kept a
  members-only post retains what they already had even after leaving (no
  clawback, consistent with [ADR-0003](0003-membership-lifecycle.md)).
- **Media/blobs** use the existing pipeline: per-blob keys sealed inside the
  encrypted payload for members-only groups; plaintext blobs for public groups.
  No new blob machinery.
