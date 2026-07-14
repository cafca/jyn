//! The Groups subsystem: hard containers people create, join, and post into,
//! each with its own membership, fixed Content mode, and replication topic.
//!
//! Standalone from the per-profile `JynSpaces` module (ADR-0004). This module
//! holds the domain types and the pure reduction of a group's operation log;
//! `service` drives the p2panda-auth/spaces layer; the sync service carries
//! group topics.
//!
//! Vocabulary follows `CONTEXT.md`: Group, Owner, Member, Join mode, Content
//! mode, Discoverability, GroupId, Group place, Groups hub, Group admin,
//! Digest door.

pub mod reduce;
pub mod service;

use serde::{Deserialize, Serialize};

pub use reduce::{
    read_group_state, GroupComment, GroupHeart, GroupJoinRequest, GroupMemberEntry,
    MembershipRecord, ReducedGroupState,
};
pub use service::{
    ensure_groups_tables, viewer_filtered, GroupSuggestion, GroupView, GroupViewerStatus,
    GroupsIngestReport, GroupsOutbox, JynGroups,
};

/// The fixed visibility of a Group's posts (ADR-0002). Immutable after
/// creation (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupContentMode {
    /// Plaintext, readable by anyone.
    Public,
    /// Encrypted to the members; unreadable to non-members and passive peers.
    MembersOnly,
}

/// How a person becomes a Member (ADR-0002, ADR-0005). Owner-mutable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupJoinMode {
    /// The Owner's node auto-accepts join requests.
    Open,
    /// The Owner approves each request manually.
    Request,
}

/// Whether members may advertise their membership to their friends
/// (ADR-0008). Owner-mutable; default listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupDiscoverability {
    #[default]
    Listed,
    Unlisted,
}

/// A role held by a Member. Membership entries carry a *set* of roles
/// (ADR-0001, ADR-0014); phase one populates only these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    /// Governs the group: the sole `Manage` holder in phase one.
    Owner,
    /// Posts and reads: `Write`.
    Member,
}

/// What a member may do, derived from their roles. Every permission check
/// routes through [`permitted_actions`] — never through an owner boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupPermission {
    /// Post, comment, and heart within the group.
    Write,
    /// Govern membership and metadata.
    Manage,
}

/// The union of permissions granted by a set of held roles (ADR-0014).
pub fn permitted_actions(roles: &[GroupRole]) -> std::collections::HashSet<GroupPermission> {
    roles
        .iter()
        .flat_map(|role| match role {
            GroupRole::Owner => &[GroupPermission::Manage][..],
            GroupRole::Member => &[GroupPermission::Write][..],
        })
        .copied()
        .collect()
}

/// Governance and membership operations — an extensible, versioned op set
/// (ADR-0014): future moderation or role ops must be addable without a
/// schema break, which the serde tag plus jyn's skip-undecodable reduction
/// rule provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GroupGovernanceAction {
    AddMember {
        member_profile_id: String,
        roles: Vec<GroupRole>,
    },
    RemoveMember {
        member_profile_id: String,
    },
    /// Replaces a member's role set; ownership transfer is a promote of the
    /// new Owner followed by a demote of the old one.
    SetMemberRoles {
        member_profile_id: String,
        roles: Vec<GroupRole>,
    },
    /// Partial metadata edit: `None` leaves a field untouched. Content mode
    /// is deliberately absent — it is immutable after creation (ADR-0006).
    EditMetadata {
        name: Option<String>,
        join_mode: Option<GroupJoinMode>,
        discoverability: Option<GroupDiscoverability>,
    },
}
