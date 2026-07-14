//! End-to-end members-only Group flow over a real relay (ADR-0002, ADR-0003,
//! ADR-0015): posts seal to the group's space, admission delivers the secret
//! via the welcome payload — friendship never enters the picture — and
//! removal re-keys the next post away from the removed member.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, GroupPostDraft, MediaDraft, NetworkCommand, NetworkEvent};
use jyn::domain::Visibility;
use jyn::groups::{GroupContentMode, GroupDiscoverability, GroupJoinMode, GroupViewerStatus};
use jyn::node::NodeOptions;
use tempfile::tempdir;

const EVENT_TIMEOUT: Duration = Duration::from_secs(120);

async fn wait_for_event<T>(
    bridge: &AsyncBridge,
    what: &str,
    mut select: impl FnMut(&NetworkEvent) -> Option<T>,
) -> Result<T> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            for event in bridge.drain_events() {
                if let Some(value) = select(&event) {
                    return value;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {what}"))
}

async fn spawn_onboarded(
    data_dir: &std::path::Path,
    node_options: NodeOptions,
    name: &str,
) -> Result<(AsyncBridge, String)> {
    let bridge = AsyncBridge::spawn_with_data_dir(node_options, data_dir.to_path_buf())?;
    let profile_id = wait_for_event(&bridge, "initial ProfileLoaded", |event| match event {
        NetworkEvent::ProfileLoaded { profile } => Some(profile.profile_id.clone()),
        _ => None,
    })
    .await?;

    bridge.send(NetworkCommand::UpdateProfile {
        display_name: name.to_owned(),
        bio: String::new(),
        default_visibility: Visibility::Friends,
        default_lifetime_secs: None,
        mark_onboarded: true,
    })?;
    wait_for_event(&bridge, "onboarded ProfileLoaded", |event| match event {
        NetworkEvent::ProfileLoaded { profile } if profile.onboarded => Some(()),
        _ => None,
    })
    .await?;

    Ok((bridge, profile_id))
}

/// Publishes `body` into the group repeatedly (key distribution may still be
/// in flight) until `reader`'s view of the group shows it.
async fn post_until_visible(
    author: &AsyncBridge,
    reader: &AsyncBridge,
    group_id: &str,
    body: &str,
    what: &str,
) -> Result<()> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            author.send(NetworkCommand::PublishGroupPost {
                group_id: group_id.to_owned(),
                draft: GroupPostDraft {
                    body: body.into(),
                    lifetime_secs: None,
                    media: Vec::new(),
                },
            })?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while tokio::time::Instant::now() < deadline {
                for event in reader.drain_events() {
                    if let NetworkEvent::GroupUpdated { view } = event {
                        if view.group_id == group_id
                            && view.posts.iter().any(|post| post.body == body)
                        {
                            return anyhow::Ok(());
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {what}"))?
}

#[tokio::test(flavor = "multi_thread")]
async fn members_only_posts_reach_members_and_rekey_on_removal() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3");
    let (_relay_map, relay_url, _relay_server) = run_relay_server().await?;
    let node_options = NodeOptions {
        relay_url: Some(relay_url.clone()),
        mdns_enabled: false,
        insecure_skip_relay_cert_verify: true,
        // Off so nodes can reopen the same data dir in-process (GC keeps the
        // blob store resident).
        gc_enabled: false,
    };

    let dir_a = tempdir()?;
    let dir_b = tempdir()?;
    let dir_c = tempdir()?;
    let (bridge_a, alice_id) = spawn_onboarded(dir_a.path(), node_options.clone(), "Alice").await?;
    let (bridge_b, _bob_id) = spawn_onboarded(dir_b.path(), node_options.clone(), "Bob").await?;
    let (bridge_c, carol_id) = spawn_onboarded(dir_c.path(), node_options.clone(), "Carol").await?;

    // Alice creates a members-only, open-join group. Nobody here is friends
    // with anybody — reach and keys travel via the group topic and the
    // members' profile topics (ADR-0015).
    bridge_a.send(NetworkCommand::CreateGroup {
        name: "sealed letters".into(),
        content_mode: GroupContentMode::MembersOnly,
        join_mode: GroupJoinMode::Open,
        discoverability: GroupDiscoverability::Listed,
    })?;
    let group_id = wait_for_event(&bridge_a, "group creation", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.name == "sealed letters" => {
            Some(view.group_id.clone())
        }
        _ => None,
    })
    .await?;

    // Bob joins; admission (welcome payload) delivers the group secret.
    bridge_b.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: None,
        via_profile_ids: vec![alice_id.clone()],
    })?;
    wait_for_event(&bridge_b, "Bob to become a member", |event| match event {
        NetworkEvent::GroupUpdated { view }
            if view.group_id == group_id && view.viewer_status == GroupViewerStatus::Member =>
        {
            Some(())
        }
        _ => None,
    })
    .await?;
    post_until_visible(
        &bridge_a,
        &bridge_b,
        &group_id,
        "first sealed note",
        "Bob to decrypt the sealed post",
    )
    .await?;

    // Carol merely observes the topic: she sees the group's identity but
    // neither content nor roster (ADR-0002; ciphertext-only for passive
    // peers).
    bridge_c.send(NetworkCommand::SyncGroup {
        group_id: group_id.clone(),
        via_profile_ids: vec![alice_id.clone()],
    })?;
    let carol_view = wait_for_event(&bridge_c, "Carol's outside view", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.group_id == group_id => Some(view.clone()),
        _ => None,
    })
    .await?;
    assert_eq!(carol_view.viewer_status, GroupViewerStatus::NonMember);
    assert_eq!(carol_view.name, "sealed letters");
    assert!(carol_view.posts.is_empty(), "no content for non-members");
    assert!(carol_view.members.is_empty(), "no roster for non-members");
    assert_eq!(carol_view.member_count, None);
    // Give the sealed post time to replicate to her node; it must never
    // decrypt for her.
    tokio::time::sleep(Duration::from_secs(5)).await;
    for event in bridge_c.drain_events() {
        if let NetworkEvent::GroupUpdated { view } = event {
            if view.group_id == group_id {
                assert!(
                    view.posts.is_empty(),
                    "a non-member must never read members-only content"
                );
            }
        }
    }

    // Carol joins and gains read access to newly sealed posts.
    bridge_c.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: None,
        via_profile_ids: vec![alice_id.clone()],
    })?;
    wait_for_event(&bridge_c, "Carol to become a member", |event| match event {
        NetworkEvent::GroupUpdated { view }
            if view.group_id == group_id && view.viewer_status == GroupViewerStatus::Member =>
        {
            Some(())
        }
        _ => None,
    })
    .await?;
    post_until_visible(
        &bridge_a,
        &bridge_c,
        &group_id,
        "second sealed note",
        "Carol to decrypt after admission",
    )
    .await?;

    // Sealed media: the blob replicates as ciphertext; a member fetches the
    // plaintext back through the per-blob key inside the sealed payload.
    let photo_path = dir_a.path().join("letter.png");
    let photo_bytes = b"sealed-bytes-are-still-bytes".to_vec();
    std::fs::write(&photo_path, &photo_bytes)?;
    bridge_a.send(NetworkCommand::PublishGroupPost {
        group_id: group_id.clone(),
        draft: GroupPostDraft {
            body: "the letter itself".into(),
            lifetime_secs: None,
            media: vec![MediaDraft {
                path: photo_path,
                kind: jyn::domain::MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;
    let (blob_hash, blob_secret) = wait_for_event(
        &bridge_b,
        "sealed post with attachment",
        |event| match event {
            NetworkEvent::GroupUpdated { view } if view.group_id == group_id => view
                .posts
                .iter()
                .find(|post| post.body == "the letter itself")
                .and_then(|post| post.media.first())
                .map(|attachment| (attachment.blob_hash.clone(), attachment.blob_secret.clone())),
            _ => None,
        },
    )
    .await?;
    assert!(
        blob_secret.is_some(),
        "a members-only attachment must carry its sealed per-blob key"
    );
    bridge_b.send(NetworkCommand::FetchMedia {
        blob_hash: blob_hash.clone(),
    })?;
    let fetched_path = wait_for_event(&bridge_b, "sealed media to arrive", |event| match event {
        NetworkEvent::MediaReady {
            blob_hash: ready_hash,
            path,
        } if *ready_hash == blob_hash => Some(path.clone()),
        _ => None,
    })
    .await?;
    assert_eq!(std::fs::read(fetched_path)?, photo_bytes);

    // Removal: the next sealed post re-keys Carol out. Bob still reads it;
    // Carol must not, and the roster closes to her again.
    bridge_a.send(NetworkCommand::RemoveGroupMember {
        group_id: group_id.clone(),
        member_profile_id: carol_id.clone(),
    })?;
    wait_for_event(&bridge_c, "Carol to see her removal", |event| match event {
        NetworkEvent::GroupUpdated { view }
            if view.group_id == group_id && view.viewer_status == GroupViewerStatus::NonMember =>
        {
            Some(())
        }
        _ => None,
    })
    .await?;
    post_until_visible(
        &bridge_a,
        &bridge_b,
        &group_id,
        "the ring tightens",
        "post-removal sealed post at Bob",
    )
    .await?;
    // Bob has it, so the ciphertext is on the topic; give Carol's node a
    // moment to ingest it, then assert it never decrypted for her.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let mut carol_sees_removed_post = false;
    for event in bridge_c.drain_events() {
        if let NetworkEvent::GroupUpdated { view } = event {
            if view.group_id == group_id
                && view
                    .posts
                    .iter()
                    .any(|post| post.body == "the ring tightens")
            {
                carol_sees_removed_post = true;
            }
        }
    }
    assert!(
        !carol_sees_removed_post,
        "a sealed post published after removal must not decrypt for the removed member"
    );

    Ok(())
}
