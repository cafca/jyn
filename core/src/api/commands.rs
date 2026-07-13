//! User actions as awaitable calls: each resolves once the runtime finished
//! the command, and throws with the failure otherwise.

use std::path::PathBuf;

use anyhow::Result;

use crate::bridge::{MediaDraft, NetworkCommand, PostDraft};
use crate::domain::Visibility;
use crate::runtime::AppRuntime;

/// A file staged on the composer. Voice notes carry the duration and
/// waveform from [`crate::api::media::voice_note_summary`]; other files
/// leave them empty. The media kind is derived from the file extension.
#[derive(Debug, Clone)]
pub struct MediaDraftInput {
    pub path: String,
    pub duration_ms: Option<u64>,
    pub waveform: Option<Vec<u8>>,
}

impl MediaDraftInput {
    fn into_draft(self) -> MediaDraft {
        let path = PathBuf::from(self.path);
        MediaDraft {
            kind: crate::media::classify(&path),
            path,
            duration_ms: self.duration_ms,
            waveform: self.waveform,
        }
    }
}

async fn run(command: NetworkCommand) -> Result<()> {
    AppRuntime::get()?.run_command(command).await
}

/// Casts a post. `lifetime_secs` counts from now; `None` is permanent.
pub async fn publish_post(
    body: String,
    visibility: Visibility,
    lifetime_secs: Option<u64>,
    media: Vec<MediaDraftInput>,
) -> Result<()> {
    run(NetworkCommand::PublishPost {
        draft: PostDraft {
            body,
            visibility,
            lifetime_secs,
            media: media.into_iter().map(MediaDraftInput::into_draft).collect(),
        },
    })
    .await
}

/// Edits a post: the body plus the full attachment list — `kept_media` are
/// the surviving originals (removed ones simply absent), `new_media` fresh
/// files to stage and append.
pub async fn edit_post(
    post_id: String,
    body: String,
    kept_media: Vec<crate::domain::MediaAttachment>,
    new_media: Vec<MediaDraftInput>,
) -> Result<()> {
    run(NetworkCommand::EditPost {
        post_id,
        body,
        kept_media,
        new_media: new_media
            .into_iter()
            .map(MediaDraftInput::into_draft)
            .collect(),
    })
    .await
}

pub async fn delete_post(post_id: String) -> Result<()> {
    run(NetworkCommand::DeletePost { post_id }).await
}

/// Promote to permanent (`None`) or let it go again (`Some(unix_secs)`).
pub async fn set_post_lifetime(post_id: String, expires_at: Option<u64>) -> Result<()> {
    run(NetworkCommand::SetPostLifetime {
        post_id,
        expires_at,
    })
    .await
}

pub async fn update_profile(
    display_name: String,
    bio: String,
    default_visibility: Visibility,
    default_lifetime_secs: Option<u64>,
    mark_onboarded: bool,
) -> Result<()> {
    run(NetworkCommand::UpdateProfile {
        display_name,
        bio,
        default_visibility,
        default_lifetime_secs,
        mark_onboarded,
    })
    .await
}

/// The share-code ritual: decode a `jyn-` friend code, reach out, and place
/// a friendship request on the target's topic.
pub async fn request_friendship(friend_code: String, greeting: Option<String>) -> Result<()> {
    run(NetworkCommand::RequestFriendship {
        friend_code,
        greeting,
    })
    .await
}

/// In-app request to a profile already put in front of us (a ghost card).
pub async fn request_friendship_by_id(profile_id: String, greeting: Option<String>) -> Result<()> {
    run(NetworkCommand::RequestFriendshipById {
        profile_id,
        greeting,
    })
    .await
}

/// Answer a pending request. Accepting follows the requester back and
/// starts syncing their stream.
pub async fn respond_friendship(requester_profile_id: String, accept: bool) -> Result<()> {
    run(NetworkCommand::RespondFriendship {
        requester_profile_id,
        accept,
    })
    .await
}

pub async fn remove_friend(profile_id: String) -> Result<()> {
    run(NetworkCommand::RemoveFriend { profile_id }).await
}

/// Toggle a named heart on someone's post.
pub async fn set_heart(
    post_author_profile_id: String,
    post_id: String,
    active: bool,
) -> Result<()> {
    run(NetworkCommand::SetHeart {
        post_author_profile_id,
        post_id,
        active,
    })
    .await
}

pub async fn publish_comment(
    post_author_profile_id: String,
    post_id: String,
    body: String,
) -> Result<()> {
    run(NetworkCommand::PublishComment {
        post_author_profile_id,
        post_id,
        body,
    })
    .await
}

/// Keep a private copy of a post — a lease that dies with the post's
/// lifetime or the author's delete.
pub async fn keep_post(post_author_profile_id: String, post_id: String) -> Result<()> {
    run(NetworkCommand::KeepPost {
        post_author_profile_id,
        post_id,
    })
    .await
}

pub async fn release_keep(post_author_profile_id: String, post_id: String) -> Result<()> {
    run(NetworkCommand::ReleaseKeep {
        post_author_profile_id,
        post_id,
    })
    .await
}

/// Writes an encrypted backup of identity-critical state (posts, group
/// encryption keys, private posts, keeps) to `dest_path`. Only the recovery
/// phrase can decrypt it.
pub async fn export_backup(dest_path: String) -> Result<()> {
    run(NetworkCommand::ExportBackup { dest_path }).await
}

// ---- Groups ----

/// Creates a Group; the caller becomes its Owner. Content mode is fixed
/// forever at creation.
pub async fn create_group(
    name: String,
    content_mode: crate::groups::GroupContentMode,
    join_mode: crate::groups::GroupJoinMode,
    discoverability: crate::groups::GroupDiscoverability,
) -> Result<()> {
    run(NetworkCommand::CreateGroup {
        name,
        content_mode,
        join_mode,
        discoverability,
    })
    .await
}

/// Joins a group's topic without membership — the visit-only read path for
/// public groups. `via_profile_ids` seed reach (the friend whose suggestion
/// or heart pointed here, or the Owner).
pub async fn sync_group(group_id: String, via_profile_ids: Vec<String>) -> Result<()> {
    run(NetworkCommand::SyncGroup {
        group_id,
        via_profile_ids,
    })
    .await
}

/// Asks to join a group. Open groups admit automatically once the Owner's
/// node processes it; request-to-join groups stay pending until answered.
pub async fn join_group(
    group_id: String,
    greeting: Option<String>,
    via_profile_ids: Vec<String>,
) -> Result<()> {
    run(NetworkCommand::JoinGroup {
        group_id,
        greeting,
        via_profile_ids,
    })
    .await
}

/// Owner answers a pending join request. Declining is local-only — a
/// declined request is never a public record.
pub async fn respond_group_join(
    group_id: String,
    requester_profile_id: String,
    accept: bool,
) -> Result<()> {
    run(NetworkCommand::RespondGroupJoin {
        group_id,
        requester_profile_id,
        accept,
    })
    .await
}

/// Casts a post into a Group. No visibility choice — the Group's Content
/// mode is the fixed visibility; lifetime stays per-post.
pub async fn publish_group_post(
    group_id: String,
    body: String,
    lifetime_secs: Option<u64>,
    media: Vec<MediaDraftInput>,
) -> Result<()> {
    run(NetworkCommand::PublishGroupPost {
        group_id,
        draft: crate::bridge::GroupPostDraft {
            body,
            lifetime_secs,
            media: media.into_iter().map(MediaDraftInput::into_draft).collect(),
        },
    })
    .await
}

pub async fn edit_group_post(
    group_id: String,
    post_id: String,
    body: String,
    kept_media: Vec<crate::domain::MediaAttachment>,
    new_media: Vec<MediaDraftInput>,
) -> Result<()> {
    run(NetworkCommand::EditGroupPost {
        group_id,
        post_id,
        body,
        kept_media,
        new_media: new_media
            .into_iter()
            .map(MediaDraftInput::into_draft)
            .collect(),
    })
    .await
}

pub async fn delete_group_post(group_id: String, post_id: String) -> Result<()> {
    run(NetworkCommand::DeleteGroupPost { group_id, post_id }).await
}

pub async fn set_group_post_lifetime(
    group_id: String,
    post_id: String,
    expires_at: Option<u64>,
) -> Result<()> {
    run(NetworkCommand::SetGroupPostLifetime {
        group_id,
        post_id,
        expires_at,
    })
    .await
}

/// Owner edits name / Join mode / Discoverability; `None` leaves a field
/// untouched. Content mode has no edit path.
pub async fn edit_group_metadata(
    group_id: String,
    name: Option<String>,
    join_mode: Option<crate::groups::GroupJoinMode>,
    discoverability: Option<crate::groups::GroupDiscoverability>,
) -> Result<()> {
    run(NetworkCommand::EditGroupMetadata {
        group_id,
        name,
        join_mode,
        discoverability,
    })
    .await
}

pub async fn remove_group_member(group_id: String, member_profile_id: String) -> Result<()> {
    run(NetworkCommand::RemoveGroupMember {
        group_id,
        member_profile_id,
    })
    .await
}

/// Moves the `Manage` role to another Member; the old Owner stays a plain
/// Member until they leave.
pub async fn transfer_group_ownership(group_id: String, to_profile_id: String) -> Result<()> {
    run(NetworkCommand::TransferGroupOwnership {
        group_id,
        to_profile_id,
    })
    .await
}

pub async fn leave_group(group_id: String) -> Result<()> {
    run(NetworkCommand::LeaveGroup { group_id }).await
}

/// Toggle a named heart on a group post (in-group).
pub async fn set_group_heart(
    group_id: String,
    post_author_profile_id: String,
    post_id: String,
    active: bool,
) -> Result<()> {
    run(NetworkCommand::SetGroupHeart {
        group_id,
        post_author_profile_id,
        post_id,
        active,
    })
    .await
}

pub async fn publish_group_comment(
    group_id: String,
    post_author_profile_id: String,
    post_id: String,
    body: String,
) -> Result<()> {
    run(NetworkCommand::PublishGroupComment {
        group_id,
        post_author_profile_id,
        post_id,
        body,
    })
    .await
}

/// Records that the viewer opened the group place, clearing its river door.
pub async fn mark_group_opened(group_id: String) -> Result<()> {
    run(NetworkCommand::MarkGroupOpened { group_id }).await
}
