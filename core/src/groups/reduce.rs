//! Pure reduction of a Group's operation log to its current state.
//!
//! Mirrors the profile reducer in `crate::domain`: deterministic over the
//! sorted operation history, with authorship enforced during reduction —
//! governance only counts from the member holding `Manage` *at that point in
//! the log*, posts and interactions only from members holding `Write`, and a
//! leave only from the member it names. Membership is an append-only log of
//! operations, never mutable boolean state (ADR-0002, ADR-0014).

use std::collections::HashMap;

use anyhow::Result;

use crate::domain::{DomainOperation, JynOperationDomain, ReducedPost, StoredDomainOperation};

use super::{
    permitted_actions, GroupContentMode, GroupDiscoverability, GroupGovernanceAction,
    GroupJoinMode, GroupPermission, GroupRole,
};

/// A current member and the set of roles they hold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupMemberEntry {
    pub profile_id: String,
    pub roles: Vec<GroupRole>,
    /// When this membership began (unix seconds).
    pub since: u64,
}

impl GroupMemberEntry {
    pub fn permissions(&self) -> std::collections::HashSet<GroupPermission> {
        permitted_actions(&self.roles)
    }
}

/// One span of the auditable membership timeline: "who could read, and from
/// when" (ADR-0002). A member who rejoins gets a new record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MembershipRecord {
    pub profile_id: String,
    pub joined_at: u64,
    pub left_at: Option<u64>,
}

/// A join request awaiting the Owner's answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupJoinRequest {
    pub requester_profile_id: String,
    pub requester_display_name: String,
    pub greeting: Option<String>,
    pub recorded_at: u64,
}

/// A comment on a group post, by any Write-holding member.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupComment {
    pub comment_id: String,
    pub commenter_profile_id: String,
    pub post_author_profile_id: String,
    pub post_id: String,
    pub body: String,
    pub created_at: u64,
}

/// An active heart on a group post.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupHeart {
    pub hearter_profile_id: String,
    pub post_author_profile_id: String,
    pub post_id: String,
    pub recorded_at: u64,
}

/// Everything a Group's operation history reduces to.
///
/// Expired posts are *not* filtered here — reduction stays deterministic and
/// restart-safe; callers filter at read time like the river does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReducedGroupState {
    pub group_id: String,
    pub creator_profile_id: String,
    pub name: String,
    pub content_mode: GroupContentMode,
    pub join_mode: GroupJoinMode,
    pub discoverability: GroupDiscoverability,
    pub created_at: u64,
    /// Current members, each carrying their set of roles.
    pub members: Vec<GroupMemberEntry>,
    /// Requests not yet answered. Wire-derived; surfacing them only to the
    /// Owner (plus the requester's own pending state) is the caller's rule.
    pub pending_requests: Vec<GroupJoinRequest>,
    /// The append-only membership timeline, preserved across leaves and
    /// removals.
    pub membership_history: Vec<MembershipRecord>,
    pub posts: Vec<ReducedPost>,
    pub comments: Vec<GroupComment>,
    pub hearts: Vec<GroupHeart>,
    pub tombstoned_post_ids: Vec<String>,
    /// Most recent post/comment/heart time, for the river digest door.
    pub latest_activity_at: u64,
}

impl ReducedGroupState {
    pub fn member(&self, profile_id: &str) -> Option<&GroupMemberEntry> {
        self.members
            .iter()
            .find(|member| member.profile_id == profile_id)
    }

    pub fn is_member(&self, profile_id: &str) -> bool {
        self.member(profile_id).is_some()
    }

    /// The member holding `Manage` — the Owner in phase one.
    pub fn owner(&self) -> Option<&GroupMemberEntry> {
        self.members
            .iter()
            .find(|member| member.permissions().contains(&GroupPermission::Manage))
    }

    /// Whether a profile may perform an action, via the roles → permitted
    /// actions function — never an owner boolean (ADR-0014).
    pub fn permits(&self, profile_id: &str, permission: GroupPermission) -> bool {
        self.member(profile_id)
            .is_some_and(|member| member.permissions().contains(&permission))
    }

    pub fn has_pending_request_from(&self, profile_id: &str) -> bool {
        self.pending_requests
            .iter()
            .any(|request| request.requester_profile_id == profile_id)
    }

    /// Posts still alive at `now`, newest first.
    pub fn active_posts(&self, now: u64) -> impl Iterator<Item = &ReducedPost> {
        self.posts.iter().filter(move |post| !post.is_expired(now))
    }
}

/// Reduces a group's full operation history to its current state.
///
/// Returns `None` until a valid genesis is known: the operation whose hash
/// *is* the GroupId, carrying [`DomainOperation::GroupCreated`] signed by the
/// creator it names.
pub async fn read_group_state(
    domain: &JynOperationDomain,
    group_id: &str,
) -> Result<Option<ReducedGroupState>> {
    let mut operations = domain.operations_for_group(group_id).await?;
    crate::domain::sort_for_reduction(&mut operations);
    Ok(reduce_group_operations(group_id, &operations))
}

fn reduce_group_operations(
    group_id: &str,
    operations: &[StoredDomainOperation],
) -> Option<ReducedGroupState> {
    // The genesis mints the group: identity, immutable Content mode, and the
    // creator's Owner+Member roles all start here.
    let genesis = operations.iter().find_map(|op| {
        if op.header.hash().to_string() != group_id {
            return None;
        }
        let DomainOperation::GroupCreated {
            creator_profile_id,
            name,
            content_mode,
            join_mode,
            discoverability,
            created_at,
        } = &op.operation
        else {
            return None;
        };
        // A genesis claiming someone else's authorship is a forgery.
        (op.author.to_string() == *creator_profile_id).then(|| {
            (
                creator_profile_id.clone(),
                name.clone(),
                *content_mode,
                *join_mode,
                *discoverability,
                *created_at,
            )
        })
    });
    let (
        creator_profile_id,
        mut name,
        content_mode,
        mut join_mode,
        mut discoverability,
        created_at,
    ) = genesis?;

    let mut members: HashMap<String, GroupMemberEntry> = HashMap::new();
    let mut history: Vec<MembershipRecord> = Vec::new();
    let mut pending: HashMap<String, GroupJoinRequest> = HashMap::new();
    let mut posts: HashMap<String, ReducedPost> = HashMap::new();
    // post_id → the profile that deleted it. A tombstone laid before its post
    // was ever seen only counts against posts by that same author — anyone
    // can claim to delete an unseen post_id, but they can only ever silence
    // themselves.
    let mut tombstones: HashMap<String, String> = HashMap::new();
    let mut comments: HashMap<String, GroupComment> = HashMap::new();
    let mut hearts: HashMap<(String, String, String), Option<u64>> = HashMap::new();
    let mut latest_activity_at = 0u64;

    let join = |members: &mut HashMap<String, GroupMemberEntry>,
                history: &mut Vec<MembershipRecord>,
                profile_id: &str,
                roles: Vec<GroupRole>,
                at: u64| {
        let mut roles = roles;
        roles.sort();
        roles.dedup();
        members.insert(
            profile_id.to_owned(),
            GroupMemberEntry {
                profile_id: profile_id.to_owned(),
                roles,
                since: at,
            },
        );
        history.push(MembershipRecord {
            profile_id: profile_id.to_owned(),
            joined_at: at,
            left_at: None,
        });
    };
    let depart = |members: &mut HashMap<String, GroupMemberEntry>,
                  history: &mut Vec<MembershipRecord>,
                  profile_id: &str,
                  at: u64| {
        members.remove(profile_id);
        if let Some(record) = history
            .iter_mut()
            .rev()
            .find(|record| record.profile_id == profile_id && record.left_at.is_none())
        {
            record.left_at = Some(at);
        }
    };

    // The creator is the first Member and holds `Manage` from the start.
    join(
        &mut members,
        &mut history,
        &creator_profile_id,
        vec![GroupRole::Owner, GroupRole::Member],
        created_at,
    );

    let can = |members: &HashMap<String, GroupMemberEntry>,
               profile_id: &str,
               permission: GroupPermission| {
        members
            .get(profile_id)
            .is_some_and(|member| member.permissions().contains(&permission))
    };
    // Whether removing `member`'s Manage would leave the group ungoverned.
    let sole_manager = |members: &HashMap<String, GroupMemberEntry>, profile_id: &str| {
        can(members, profile_id, GroupPermission::Manage)
            && !members.values().any(|member| {
                member.profile_id != profile_id
                    && member.permissions().contains(&GroupPermission::Manage)
            })
    };

    for op in operations {
        let author_id = op.author.to_string();
        if op.header.hash().to_string() == group_id {
            continue; // The genesis was applied above.
        }

        match &op.operation {
            DomainOperation::GroupGoverned {
                group_id: target,
                actor_profile_id,
                action,
                recorded_at,
            } => {
                // Governance must come from the account it claims, and that
                // account must hold `Manage` at this point in the log.
                if target != group_id
                    || author_id != *actor_profile_id
                    || !can(&members, &author_id, GroupPermission::Manage)
                {
                    continue;
                }
                match action {
                    GroupGovernanceAction::AddMember {
                        member_profile_id,
                        roles,
                    } => {
                        if !members.contains_key(member_profile_id) {
                            join(
                                &mut members,
                                &mut history,
                                member_profile_id,
                                roles.clone(),
                                *recorded_at,
                            );
                        }
                        pending.remove(member_profile_id);
                    }
                    GroupGovernanceAction::RemoveMember { member_profile_id } => {
                        // The `Manage` holder exits by transfer, never by
                        // removal (ADR-0003).
                        if members.contains_key(member_profile_id)
                            && !can(&members, member_profile_id, GroupPermission::Manage)
                        {
                            depart(&mut members, &mut history, member_profile_id, *recorded_at);
                        }
                    }
                    GroupGovernanceAction::SetMemberRoles {
                        member_profile_id,
                        roles,
                    } => {
                        // Never let the group end up with no `Manage` holder.
                        let demotes_last_manager = sole_manager(&members, member_profile_id)
                            && !permitted_actions(roles).contains(&GroupPermission::Manage);
                        if let Some(member) = members.get_mut(member_profile_id) {
                            if !demotes_last_manager {
                                let mut roles = roles.clone();
                                roles.sort();
                                roles.dedup();
                                member.roles = roles;
                            }
                        }
                    }
                    GroupGovernanceAction::EditMetadata {
                        name: next_name,
                        join_mode: next_join_mode,
                        discoverability: next_discoverability,
                    } => {
                        if let Some(next) = next_name {
                            name = next.clone();
                        }
                        if let Some(next) = next_join_mode {
                            join_mode = *next;
                        }
                        if let Some(next) = next_discoverability {
                            discoverability = *next;
                        }
                    }
                }
            }
            DomainOperation::GroupJoinRequested {
                group_id: target,
                requester_profile_id,
                requester_display_name,
                greeting,
                recorded_at,
            } => {
                // A request must be signed by the requester it claims;
                // members have nothing to request.
                if target != group_id
                    || author_id != *requester_profile_id
                    || members.contains_key(requester_profile_id)
                {
                    continue;
                }
                pending.insert(
                    requester_profile_id.clone(),
                    GroupJoinRequest {
                        requester_profile_id: requester_profile_id.clone(),
                        requester_display_name: requester_display_name.clone(),
                        greeting: greeting.clone(),
                        recorded_at: *recorded_at,
                    },
                );
            }
            DomainOperation::GroupLeft {
                group_id: target,
                member_profile_id,
                recorded_at,
            } => {
                // Only self-authored, and the `Manage` holder cannot leave —
                // ownership transfers first (ADR-0003); a sole-owner group
                // goes dormant instead.
                if target != group_id
                    || author_id != *member_profile_id
                    || !members.contains_key(member_profile_id)
                    || can(&members, member_profile_id, GroupPermission::Manage)
                {
                    continue;
                }
                depart(&mut members, &mut history, member_profile_id, *recorded_at);
            }
            DomainOperation::PostPublished {
                profile_id,
                post_id,
                body,
                media,
                visibility,
                expires_at,
                created_at,
                edited,
            } => {
                // Posting is governed by membership (`Write`), and a post
                // must be signed by the author it claims.
                // A tombstone only suppresses a post by the same author who
                // laid it — a delete for a not-yet-seen post_id can never
                // censor another member's post.
                if author_id != *profile_id
                    || !can(&members, &author_id, GroupPermission::Write)
                    || tombstones.get(post_id) == Some(&author_id)
                {
                    continue;
                }
                latest_activity_at = latest_activity_at.max(*created_at);
                // A later publication for the same post id is a snapshot
                // (ADR-0016): a lifetime change re-publishes the post's
                // complete state, so inserting over the old copy is exact.
                posts.insert(
                    post_id.clone(),
                    ReducedPost {
                        profile_id: profile_id.clone(),
                        post_id: post_id.clone(),
                        body: body.clone(),
                        media: media.clone(),
                        visibility: *visibility,
                        expires_at: *expires_at,
                        created_at: *created_at,
                        edited: *edited,
                    },
                );
            }
            DomainOperation::PostEdited {
                profile_id,
                post_id,
                body,
                media,
                ..
            } => {
                // Author sovereignty: only the post's author edits it —
                // membership is not required (an ex-member still owns their
                // words).
                if author_id != *profile_id {
                    continue;
                }
                if let Some(post) = posts.get_mut(post_id) {
                    if post.profile_id == author_id {
                        post.body = body.clone();
                        if let Some(media) = media {
                            post.media = media.clone();
                        }
                        post.edited = true;
                    }
                }
            }
            DomainOperation::PostDeleted {
                profile_id,
                post_id,
                ..
            } => {
                if author_id != *profile_id {
                    continue;
                }
                if posts
                    .get(post_id)
                    .is_none_or(|post| post.profile_id == author_id)
                {
                    posts.remove(post_id);
                    tombstones.insert(post_id.clone(), author_id.clone());
                }
            }
            DomainOperation::CommentPublished {
                profile_id,
                comment_id,
                post_author_profile_id,
                post_id,
                body,
                created_at,
            } => {
                if author_id != *profile_id || !can(&members, &author_id, GroupPermission::Write) {
                    continue;
                }
                latest_activity_at = latest_activity_at.max(*created_at);
                comments.insert(
                    comment_id.clone(),
                    GroupComment {
                        comment_id: comment_id.clone(),
                        commenter_profile_id: profile_id.clone(),
                        post_author_profile_id: post_author_profile_id.clone(),
                        post_id: post_id.clone(),
                        body: body.clone(),
                        created_at: *created_at,
                    },
                );
            }
            DomainOperation::HeartChanged {
                profile_id,
                post_author_profile_id,
                post_id,
                active,
                recorded_at,
                ..
            } => {
                if author_id != *profile_id || !can(&members, &author_id, GroupPermission::Write) {
                    continue;
                }
                latest_activity_at = latest_activity_at.max(*recorded_at);
                hearts.insert(
                    (
                        profile_id.clone(),
                        post_author_profile_id.clone(),
                        post_id.clone(),
                    ),
                    active.then_some(*recorded_at),
                );
            }
            // Profile-topic operations have no meaning in a group context;
            // spaces wrappers were substituted or dropped before reduction.
            _ => {}
        }
    }

    let mut members: Vec<GroupMemberEntry> = members.into_values().collect();
    members.sort_by(|left, right| {
        left.since
            .cmp(&right.since)
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });

    let mut pending_requests: Vec<GroupJoinRequest> = pending.into_values().collect();
    pending_requests.sort_by(|left, right| {
        left.recorded_at
            .cmp(&right.recorded_at)
            .then_with(|| left.requester_profile_id.cmp(&right.requester_profile_id))
    });

    let mut posts: Vec<ReducedPost> = posts.into_values().collect();
    posts.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.post_id.cmp(&right.post_id))
    });

    let mut comments: Vec<GroupComment> = comments.into_values().collect();
    comments.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.comment_id.cmp(&right.comment_id))
    });

    let mut hearts: Vec<GroupHeart> = hearts
        .into_iter()
        .filter_map(|((hearter, post_author, post_id), recorded_at)| {
            recorded_at.map(|recorded_at| GroupHeart {
                hearter_profile_id: hearter,
                post_author_profile_id: post_author,
                post_id,
                recorded_at,
            })
        })
        .collect();
    hearts.sort_by(|left, right| {
        left.recorded_at
            .cmp(&right.recorded_at)
            .then_with(|| left.post_id.cmp(&right.post_id))
            .then_with(|| left.hearter_profile_id.cmp(&right.hearter_profile_id))
    });

    let mut tombstoned_post_ids: Vec<String> = tombstones.into_keys().collect();
    tombstoned_post_ids.sort();

    Some(ReducedGroupState {
        group_id: group_id.to_owned(),
        creator_profile_id,
        name,
        content_mode,
        join_mode,
        discoverability,
        created_at,
        members,
        pending_requests,
        membership_history: history,
        posts,
        comments,
        hearts,
        tombstoned_post_ids,
        latest_activity_at,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use p2panda_core::SigningKey;
    use p2panda_store::SqliteStore;

    use super::*;
    use crate::domain::Visibility;

    struct GroupFixture {
        domain: JynOperationDomain,
        owner_key: SigningKey,
        owner_id: String,
        group_id: String,
    }

    async fn public_group(join_mode: GroupJoinMode) -> Result<GroupFixture> {
        let owner_key = SigningKey::generate();
        let owner_id = owner_key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);
        let header = domain
            .append_group_operation(
                &owner_key,
                None,
                DomainOperation::GroupCreated {
                    creator_profile_id: owner_id.clone(),
                    name: "reading circle".into(),
                    content_mode: GroupContentMode::Public,
                    join_mode,
                    discoverability: GroupDiscoverability::Listed,
                    created_at: 10,
                },
            )
            .await?;
        Ok(GroupFixture {
            domain,
            owner_key,
            owner_id,
            group_id: header.hash().to_string(),
        })
    }

    impl GroupFixture {
        async fn govern(&mut self, action: GroupGovernanceAction, at: u64) -> Result<()> {
            self.govern_as(&self.owner_key.clone(), &self.owner_id.clone(), action, at)
                .await
        }

        async fn govern_as(
            &mut self,
            key: &SigningKey,
            actor_id: &str,
            action: GroupGovernanceAction,
            at: u64,
        ) -> Result<()> {
            let group_id = self.group_id.clone();
            self.domain
                .append_group_operation(
                    key,
                    Some(&group_id),
                    DomainOperation::GroupGoverned {
                        group_id: group_id.clone(),
                        actor_profile_id: actor_id.to_owned(),
                        action,
                        recorded_at: at,
                    },
                )
                .await?;
            Ok(())
        }

        async fn add_member(&mut self, member_id: &str, at: u64) -> Result<()> {
            self.govern(
                GroupGovernanceAction::AddMember {
                    member_profile_id: member_id.to_owned(),
                    roles: vec![GroupRole::Member],
                },
                at,
            )
            .await
        }

        async fn post_as(
            &mut self,
            key: &SigningKey,
            post_id: &str,
            body: &str,
            at: u64,
        ) -> Result<()> {
            let group_id = self.group_id.clone();
            self.domain
                .append_group_operation(
                    key,
                    Some(&group_id),
                    DomainOperation::PostPublished {
                        profile_id: key.verifying_key().to_string(),
                        post_id: post_id.to_owned(),
                        body: body.to_owned(),
                        media: Vec::new(),
                        visibility: Visibility::Public,
                        expires_at: None,
                        created_at: at,
                        edited: false,
                    },
                )
                .await?;
            Ok(())
        }

        async fn state(&self) -> Result<ReducedGroupState> {
            Ok(read_group_state(&self.domain, &self.group_id)
                .await?
                .expect("group state exists"))
        }
    }

    #[tokio::test]
    async fn genesis_mints_the_group_with_the_creator_as_owner_and_member() -> Result<()> {
        let fixture = public_group(GroupJoinMode::Open).await?;
        let state = fixture.state().await?;

        assert_eq!(state.name, "reading circle");
        assert_eq!(state.content_mode, GroupContentMode::Public);
        assert_eq!(state.join_mode, GroupJoinMode::Open);
        assert_eq!(state.discoverability, GroupDiscoverability::Listed);
        assert_eq!(state.members.len(), 1);
        let creator = &state.members[0];
        assert_eq!(creator.profile_id, fixture.owner_id);
        assert_eq!(creator.roles, vec![GroupRole::Owner, GroupRole::Member]);
        // Authority routes through roles → permitted actions, never a flag.
        assert!(state.permits(&fixture.owner_id, GroupPermission::Manage));
        assert!(state.permits(&fixture.owner_id, GroupPermission::Write));
        assert_eq!(state.owner().unwrap().profile_id, fixture.owner_id);
        Ok(())
    }

    #[tokio::test]
    async fn forged_genesis_claiming_someone_else_reduces_to_nothing() -> Result<()> {
        let attacker_key = SigningKey::generate();
        let victim_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);
        let header = domain
            .append_group_operation(
                &attacker_key,
                None,
                DomainOperation::GroupCreated {
                    creator_profile_id: victim_id,
                    name: "not yours".into(),
                    content_mode: GroupContentMode::Public,
                    join_mode: GroupJoinMode::Open,
                    discoverability: GroupDiscoverability::Listed,
                    created_at: 10,
                },
            )
            .await?;

        let state = read_group_state(&domain, &header.hash().to_string()).await?;
        assert!(state.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn join_request_pends_until_the_owner_adds_the_member() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Request).await?;
        let joiner_key = SigningKey::generate();
        let joiner_id = joiner_key.verifying_key().to_string();

        let group_id = fixture.group_id.clone();
        fixture
            .domain
            .append_group_operation(
                &joiner_key,
                Some(&group_id),
                DomainOperation::GroupJoinRequested {
                    group_id: group_id.clone(),
                    requester_profile_id: joiner_id.clone(),
                    requester_display_name: "Wen Li".into(),
                    greeting: Some("saw this through Anna".into()),
                    recorded_at: 20,
                },
            )
            .await?;

        let state = fixture.state().await?;
        assert_eq!(state.pending_requests.len(), 1);
        assert_eq!(state.pending_requests[0].requester_profile_id, joiner_id);
        assert!(state.has_pending_request_from(&joiner_id));
        assert!(!state.is_member(&joiner_id));

        fixture.add_member(&joiner_id, 30).await?;
        let state = fixture.state().await?;
        assert!(state.pending_requests.is_empty());
        let member = state.member(&joiner_id).expect("joiner is a member");
        assert_eq!(member.roles, vec![GroupRole::Member]);
        assert!(state.permits(&joiner_id, GroupPermission::Write));
        assert!(!state.permits(&joiner_id, GroupPermission::Manage));
        Ok(())
    }

    #[tokio::test]
    async fn governance_from_a_non_manage_member_is_ignored() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_key = SigningKey::generate();
        let member_id = member_key.verifying_key().to_string();
        let stranger_id = SigningKey::generate().verifying_key().to_string();
        fixture.add_member(&member_id, 20).await?;

        // A Write-only member tries to add someone and to rename the group.
        fixture
            .govern_as(
                &member_key,
                &member_id,
                GroupGovernanceAction::AddMember {
                    member_profile_id: stranger_id.clone(),
                    roles: vec![GroupRole::Member],
                },
                30,
            )
            .await?;
        fixture
            .govern_as(
                &member_key,
                &member_id,
                GroupGovernanceAction::EditMetadata {
                    name: Some("hijacked".into()),
                    join_mode: None,
                    discoverability: None,
                },
                31,
            )
            .await?;

        let state = fixture.state().await?;
        assert!(!state.is_member(&stranger_id));
        assert_eq!(state.name, "reading circle");
        Ok(())
    }

    #[tokio::test]
    async fn posts_are_membership_gated_and_author_owned() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_key = SigningKey::generate();
        let member_id = member_key.verifying_key().to_string();
        let stranger_key = SigningKey::generate();
        fixture.add_member(&member_id, 20).await?;

        let owner_key = fixture.owner_key.clone();
        fixture
            .post_as(&member_key, "post-m", "from the member", 30)
            .await?;
        fixture
            .post_as(&owner_key, "post-o", "from the owner", 31)
            .await?;
        fixture
            .post_as(&stranger_key, "post-s", "from outside", 32)
            .await?;

        let state = fixture.state().await?;
        let ids: Vec<&str> = state
            .posts
            .iter()
            .map(|post| post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["post-o", "post-m"]);
        assert_eq!(state.latest_activity_at, 31);

        // Only the author edits or deletes their post.
        let group_id = fixture.group_id.clone();
        fixture
            .domain
            .append_group_operation(
                &owner_key,
                Some(&group_id),
                DomainOperation::PostDeleted {
                    profile_id: fixture.owner_id.clone(),
                    post_id: "post-m".into(),
                    deleted_at: 40,
                },
            )
            .await?;
        let state = fixture.state().await?;
        assert!(state.posts.iter().any(|post| post.post_id == "post-m"));

        fixture
            .domain
            .append_group_operation(
                &member_key,
                Some(&group_id),
                DomainOperation::PostDeleted {
                    profile_id: member_id.clone(),
                    post_id: "post-m".into(),
                    deleted_at: 41,
                },
            )
            .await?;
        let state = fixture.state().await?;
        assert!(state.posts.iter().all(|post| post.post_id != "post-m"));
        assert!(state.tombstoned_post_ids.contains(&"post-m".to_owned()));
        Ok(())
    }

    #[tokio::test]
    async fn a_delete_cannot_censor_another_members_post() -> Result<()> {
        // An attacker who learns a post_id must not be able to pre-empt it:
        // a PostDeleted laid before the post is seen only suppresses a post by
        // that same author, never the real author's post.
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let attacker_key = SigningKey::generate();
        let attacker_id = attacker_key.verifying_key().to_string();
        let victim_key = SigningKey::generate();
        let victim_id = victim_key.verifying_key().to_string();
        fixture.add_member(&attacker_id, 20).await?;
        fixture.add_member(&victim_id, 21).await?;

        // The attacker tombstones a post_id that has not yet been published —
        // sorting before the victim's post because it is appended first.
        let group_id = fixture.group_id.clone();
        fixture
            .domain
            .append_group_operation(
                &attacker_key,
                Some(&group_id),
                DomainOperation::PostDeleted {
                    profile_id: attacker_id.clone(),
                    post_id: "victim-post".into(),
                    deleted_at: 30,
                },
            )
            .await?;
        fixture
            .post_as(&victim_key, "victim-post", "my words stand", 31)
            .await?;

        let state = fixture.state().await?;
        assert!(
            state.posts.iter().any(|post| post.post_id == "victim-post"),
            "a foreign tombstone must not suppress the author's real post"
        );

        // But an author's own delete-before-publish (out-of-order replication)
        // still suppresses their post.
        fixture
            .domain
            .append_group_operation(
                &attacker_key,
                Some(&group_id),
                DomainOperation::PostDeleted {
                    profile_id: attacker_id.clone(),
                    post_id: "own-post".into(),
                    deleted_at: 32,
                },
            )
            .await?;
        fixture
            .post_as(&attacker_key, "own-post", "never mind", 33)
            .await?;
        let state = fixture.state().await?;
        assert!(
            state.posts.iter().all(|post| post.post_id != "own-post"),
            "an author's own delete-before-publish still holds"
        );
        Ok(())
    }

    #[tokio::test]
    async fn removal_ends_membership_and_later_posts_are_dropped() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_key = SigningKey::generate();
        let member_id = member_key.verifying_key().to_string();
        fixture.add_member(&member_id, 20).await?;
        fixture
            .post_as(&member_key, "before", "still in", 25)
            .await?;

        fixture
            .govern(
                GroupGovernanceAction::RemoveMember {
                    member_profile_id: member_id.clone(),
                },
                30,
            )
            .await?;
        fixture
            .post_as(&member_key, "after", "locked out", 35)
            .await?;

        let state = fixture.state().await?;
        assert!(!state.is_member(&member_id));
        let ids: Vec<&str> = state
            .posts
            .iter()
            .map(|post| post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["before"]);
        // The auditable timeline keeps the span.
        let record = state
            .membership_history
            .iter()
            .find(|record| record.profile_id == member_id)
            .expect("membership span recorded");
        assert_eq!(record.joined_at, 20);
        assert_eq!(record.left_at, Some(30));
        Ok(())
    }

    #[tokio::test]
    async fn the_manage_holder_cannot_be_removed_or_leave() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_id = SigningKey::generate().verifying_key().to_string();
        fixture.add_member(&member_id, 20).await?;

        // Remove-self and leave are both ignored for the Manage holder.
        let owner_id = fixture.owner_id.clone();
        fixture
            .govern(
                GroupGovernanceAction::RemoveMember {
                    member_profile_id: owner_id.clone(),
                },
                30,
            )
            .await?;
        let group_id = fixture.group_id.clone();
        let owner_key = fixture.owner_key.clone();
        fixture
            .domain
            .append_group_operation(
                &owner_key,
                Some(&group_id),
                DomainOperation::GroupLeft {
                    group_id: group_id.clone(),
                    member_profile_id: owner_id.clone(),
                    recorded_at: 31,
                },
            )
            .await?;
        // Demoting the sole Manage holder is likewise ignored.
        fixture
            .govern(
                GroupGovernanceAction::SetMemberRoles {
                    member_profile_id: owner_id.clone(),
                    roles: vec![GroupRole::Member],
                },
                32,
            )
            .await?;

        let state = fixture.state().await?;
        assert!(state.permits(&owner_id, GroupPermission::Manage));
        assert_eq!(state.owner().unwrap().profile_id, owner_id);
        Ok(())
    }

    #[tokio::test]
    async fn ownership_transfers_and_the_former_owner_can_leave() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let heir_key = SigningKey::generate();
        let heir_id = heir_key.verifying_key().to_string();
        fixture.add_member(&heir_id, 20).await?;

        // Promote the heir, then demote (transfer, ADR-0003), then leave.
        fixture
            .govern(
                GroupGovernanceAction::SetMemberRoles {
                    member_profile_id: heir_id.clone(),
                    roles: vec![GroupRole::Owner, GroupRole::Member],
                },
                30,
            )
            .await?;
        let owner_id = fixture.owner_id.clone();
        fixture
            .govern(
                GroupGovernanceAction::SetMemberRoles {
                    member_profile_id: owner_id.clone(),
                    roles: vec![GroupRole::Member],
                },
                31,
            )
            .await?;
        let group_id = fixture.group_id.clone();
        let owner_key = fixture.owner_key.clone();
        fixture
            .domain
            .append_group_operation(
                &owner_key,
                Some(&group_id),
                DomainOperation::GroupLeft {
                    group_id: group_id.clone(),
                    member_profile_id: owner_id.clone(),
                    recorded_at: 40,
                },
            )
            .await?;

        // The group persists under the heir; the creator is fully gone.
        let state = fixture.state().await?;
        assert!(!state.is_member(&owner_id));
        assert_eq!(state.owner().unwrap().profile_id, heir_id);
        assert!(state.permits(&heir_id, GroupPermission::Manage));

        // And the heir now governs: they can admit someone new.
        let newcomer_id = SigningKey::generate().verifying_key().to_string();
        fixture
            .govern_as(
                &heir_key,
                &heir_id,
                GroupGovernanceAction::AddMember {
                    member_profile_id: newcomer_id.clone(),
                    roles: vec![GroupRole::Member],
                },
                50,
            )
            .await?;
        let state = fixture.state().await?;
        assert!(state.is_member(&newcomer_id));
        Ok(())
    }

    #[tokio::test]
    async fn metadata_edits_apply_only_the_named_fields() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        fixture
            .govern(
                GroupGovernanceAction::EditMetadata {
                    name: Some("evening reading circle".into()),
                    join_mode: Some(GroupJoinMode::Request),
                    discoverability: None,
                },
                20,
            )
            .await?;

        let state = fixture.state().await?;
        assert_eq!(state.name, "evening reading circle");
        assert_eq!(state.join_mode, GroupJoinMode::Request);
        assert_eq!(state.discoverability, GroupDiscoverability::Listed);
        // Content mode has no edit path at all: fixed at creation.
        assert_eq!(state.content_mode, GroupContentMode::Public);
        Ok(())
    }

    #[tokio::test]
    async fn comments_and_hearts_are_membership_gated_and_hearts_toggle() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_key = SigningKey::generate();
        let member_id = member_key.verifying_key().to_string();
        let stranger_key = SigningKey::generate();
        let stranger_id = stranger_key.verifying_key().to_string();
        fixture.add_member(&member_id, 20).await?;
        let owner_key = fixture.owner_key.clone();
        fixture
            .post_as(&owner_key, "post-1", "hello group", 25)
            .await?;

        let group_id = fixture.group_id.clone();
        let owner_id = fixture.owner_id.clone();
        let comment =
            |commenter: &str, comment_id: &str, at: u64| DomainOperation::CommentPublished {
                profile_id: commenter.to_owned(),
                comment_id: comment_id.to_owned(),
                post_author_profile_id: owner_id.clone(),
                post_id: "post-1".into(),
                body: "nice".into(),
                created_at: at,
            };
        fixture
            .domain
            .append_group_operation(&member_key, Some(&group_id), comment(&member_id, "c-1", 30))
            .await?;
        fixture
            .domain
            .append_group_operation(
                &stranger_key,
                Some(&group_id),
                comment(&stranger_id, "c-2", 31),
            )
            .await?;

        let heart = |active: bool, at: u64| DomainOperation::HeartChanged {
            profile_id: member_id.clone(),
            post_author_profile_id: owner_id.clone(),
            post_id: "post-1".into(),
            active,
            recorded_at: at,
            group_id: None,
            group_name: None,
        };
        fixture
            .domain
            .append_group_operation(&member_key, Some(&group_id), heart(true, 40))
            .await?;
        fixture
            .domain
            .append_group_operation(&member_key, Some(&group_id), heart(false, 41))
            .await?;
        fixture
            .domain
            .append_group_operation(&member_key, Some(&group_id), heart(true, 42))
            .await?;

        let state = fixture.state().await?;
        let comment_ids: Vec<&str> = state
            .comments
            .iter()
            .map(|comment| comment.comment_id.as_str())
            .collect();
        assert_eq!(comment_ids, vec!["c-1"]);
        assert_eq!(
            state.hearts,
            vec![GroupHeart {
                hearter_profile_id: member_id,
                post_author_profile_id: owner_id,
                post_id: "post-1".into(),
                recorded_at: 42,
            }]
        );
        Ok(())
    }

    #[tokio::test]
    async fn leaving_and_rejoining_appends_a_new_membership_span() -> Result<()> {
        let mut fixture = public_group(GroupJoinMode::Open).await?;
        let member_key = SigningKey::generate();
        let member_id = member_key.verifying_key().to_string();
        fixture.add_member(&member_id, 20).await?;

        let group_id = fixture.group_id.clone();
        fixture
            .domain
            .append_group_operation(
                &member_key,
                Some(&group_id),
                DomainOperation::GroupLeft {
                    group_id: group_id.clone(),
                    member_profile_id: member_id.clone(),
                    recorded_at: 30,
                },
            )
            .await?;
        fixture.add_member(&member_id, 40).await?;

        let state = fixture.state().await?;
        assert!(state.is_member(&member_id));
        let spans: Vec<(u64, Option<u64>)> = state
            .membership_history
            .iter()
            .filter(|record| record.profile_id == member_id)
            .map(|record| (record.joined_at, record.left_at))
            .collect();
        assert_eq!(spans, vec![(20, Some(30)), (40, None)]);
        Ok(())
    }
}
