//! End-to-end public-Groups flow over a real relay, driven through the same
//! bridge the UI uses. Groups span non-friends: nobody in these tests ever
//! befriends anybody — reach comes from the group topic alone (ADR-0007).

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, GroupPostDraft, NetworkCommand, NetworkEvent};
use jyn::domain::Visibility;
use jyn::groups::{
    GroupContentMode, GroupDiscoverability, GroupJoinMode, GroupView, GroupViewerStatus,
};
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

/// Selects the latest view of one group out of an event.
fn group_view_of<'a>(event: &'a NetworkEvent, group_id: &str) -> Option<&'a GroupView> {
    match event {
        NetworkEvent::GroupUpdated { view } if view.group_id == group_id => Some(view),
        _ => None,
    }
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

fn group_post(group_id: &str, body: &str) -> NetworkCommand {
    NetworkCommand::PublishGroupPost {
        group_id: group_id.to_owned(),
        draft: GroupPostDraft {
            body: body.into(),
            lifetime_secs: None,
            media: Vec::new(),
        },
    }
}

/// The full public-group container + governance arc: create → post →
/// cross-node read (member and non-member), open join auto-accept, context
/// exclusivity (group posts never in the river), remove, and
/// transfer-then-leave with the group surviving its creator's departure.
#[tokio::test(flavor = "multi_thread")]
async fn public_group_lifecycle_over_open_join() -> Result<()> {
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
    let (bridge_b, bob_id) = spawn_onboarded(dir_b.path(), node_options.clone(), "Bob").await?;
    let (bridge_c, carol_id) = spawn_onboarded(dir_c.path(), node_options.clone(), "Carol").await?;

    // Alice creates an open public Group and posts into it.
    bridge_a.send(NetworkCommand::CreateGroup {
        name: "river watchers".into(),
        content_mode: GroupContentMode::Public,
        join_mode: GroupJoinMode::Open,
        discoverability: GroupDiscoverability::Listed,
    })?;
    let group_id = wait_for_event(&bridge_a, "group creation", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.name == "river watchers" => {
            Some(view.group_id.clone())
        }
        _ => None,
    })
    .await?;
    bridge_a.send(group_post(&group_id, "first stone in the water"))?;

    // Bob — a stranger to Alice, no friendship anywhere — visits the group
    // and can read the public stream without joining.
    bridge_b.send(NetworkCommand::SyncGroup {
        group_id: group_id.clone(),
        via_profile_ids: vec![alice_id.clone()],
    })?;
    let bob_view = wait_for_event(&bridge_b, "public read as non-member", |event| {
        group_view_of(event, &group_id)
            .filter(|view| {
                view.posts
                    .iter()
                    .any(|post| post.body == "first stone in the water")
            })
            .cloned()
    })
    .await?;
    assert_eq!(bob_view.viewer_status, GroupViewerStatus::NonMember);
    assert_eq!(bob_view.name, "river watchers");
    // A public group's roster is visible to anyone.
    assert_eq!(bob_view.member_count, Some(1));

    // Open join: Bob asks, Alice's node auto-accepts, membership becomes
    // authoritative once her node processed it.
    bridge_b.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: None,
        via_profile_ids: vec![alice_id.clone()],
    })?;
    wait_for_event(&bridge_b, "Bob to become a member", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Member)
            .map(|_| ())
    })
    .await?;

    // Bob posts into the group; Alice reads it.
    bridge_b.send(group_post(&group_id, "skipping the second stone"))?;
    let bob_post_id = wait_for_event(&bridge_a, "Bob's group post at Alice", |event| {
        group_view_of(event, &group_id).and_then(|view| {
            view.posts
                .iter()
                .find(|post| post.body == "skipping the second stone")
                .map(|post| post.post_id.clone())
        })
    })
    .await?;

    // Comments inherit the group's (public) Content mode and flow between
    // members over the group topic.
    bridge_a.send(NetworkCommand::PublishGroupComment {
        group_id: group_id.clone(),
        post_author_profile_id: bob_id.clone(),
        post_id: bob_post_id.clone(),
        body: "nice arc".into(),
    })?;
    wait_for_event(&bridge_b, "Alice's comment at Bob", |event| {
        group_view_of(event, &group_id)
            .filter(|view| {
                view.comments
                    .iter()
                    .any(|comment| comment.post_id == bob_post_id && comment.body == "nice arc")
            })
            .map(|_| ())
    })
    .await?;

    // Media attachments work on public group posts as plaintext blobs:
    // Bob fetches Alice's photo byte-identical through the group context.
    let photo_path = dir_a.path().join("stone.png");
    let photo_bytes = b"not-really-a-png-but-bytes-are-bytes".to_vec();
    std::fs::write(&photo_path, &photo_bytes)?;
    bridge_a.send(NetworkCommand::PublishGroupPost {
        group_id: group_id.clone(),
        draft: GroupPostDraft {
            body: "the stone itself".into(),
            lifetime_secs: None,
            media: vec![jyn::bridge::MediaDraft {
                path: photo_path,
                kind: jyn::domain::MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;
    let blob_hash = wait_for_event(&bridge_b, "group post with attachment", |event| {
        group_view_of(event, &group_id).and_then(|view| {
            view.posts
                .iter()
                .find(|post| post.body == "the stone itself")
                .and_then(|post| post.media.first())
                .map(|attachment| attachment.blob_hash.clone())
        })
    })
    .await?;
    bridge_b.send(NetworkCommand::FetchMedia {
        blob_hash: blob_hash.clone(),
    })?;
    let fetched_path = wait_for_event(&bridge_b, "group media to arrive", |event| match event {
        NetworkEvent::MediaReady {
            blob_hash: ready_hash,
            path,
        } if *ready_hash == blob_hash => Some(path.clone()),
        _ => None,
    })
    .await?;
    assert_eq!(std::fs::read(fetched_path)?, photo_bytes);

    // Context exclusivity: neither author's *profile* stream carries the
    // group posts — the river never sees them (ADR-0007).
    let own_posts_of = |bridge: &AsyncBridge| {
        bridge
            .drain_events()
            .into_iter()
            .find_map(|event| match event {
                NetworkEvent::LocalStateUpdated { state } => Some(state.posts),
                _ => None,
            })
    };
    let _ = own_posts_of(&bridge_a); // drain
    bridge_a.send(NetworkCommand::RecoverStartup)?;
    let alice_profile_posts =
        wait_for_event(&bridge_a, "Alice's profile state", |event| match event {
            NetworkEvent::LocalStateUpdated { state } => Some(state.posts.clone()),
            _ => None,
        })
        .await?;
    assert!(
        alice_profile_posts
            .iter()
            .all(|post| post.body != "first stone in the water"),
        "a group post must never appear in the author's profile stream"
    );

    // Carol joins too.
    bridge_c.send(NetworkCommand::SyncGroup {
        group_id: group_id.clone(),
        via_profile_ids: vec![alice_id.clone()],
    })?;
    bridge_c.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: None,
        via_profile_ids: vec![alice_id.clone()],
    })?;
    wait_for_event(&bridge_c, "Carol to become a member", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Member)
            .map(|_| ())
    })
    .await?;

    // Governance: Alice removes Carol; Carol sees herself outside again
    // (and, the group being public, can still read).
    bridge_a.send(NetworkCommand::RemoveGroupMember {
        group_id: group_id.clone(),
        member_profile_id: carol_id.clone(),
    })?;
    let carol_after_removal = wait_for_event(&bridge_c, "Carol to see her removal", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::NonMember)
            .cloned()
    })
    .await?;
    assert!(
        carol_after_removal
            .posts
            .iter()
            .any(|post| post.body == "first stone in the water"),
        "a removed member of a public group can still read it"
    );

    // Transfer-then-leave: Alice hands `Manage` to Bob and leaves; the group
    // persists under Bob (nothing mutable anchors to the creator, ADR-0006).
    bridge_a.send(NetworkCommand::TransferGroupOwnership {
        group_id: group_id.clone(),
        to_profile_id: bob_id.clone(),
    })?;
    wait_for_event(&bridge_b, "Bob to become the owner", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Owner)
            .map(|_| ())
    })
    .await?;
    bridge_a.send(NetworkCommand::LeaveGroup {
        group_id: group_id.clone(),
    })?;
    wait_for_event(&bridge_b, "Bob to see Alice gone", |event| {
        group_view_of(event, &group_id)
            .filter(|view| {
                view.owner_profile_id == bob_id
                    && !view.members.iter().any(|m| m.profile_id == alice_id)
            })
            .map(|_| ())
    })
    .await?;

    // The group lives on: Bob governs (re-admits Carol) and posting works.
    bridge_b.send(NetworkCommand::RespondGroupJoin {
        group_id: group_id.clone(),
        requester_profile_id: carol_id.clone(),
        accept: true,
    })?;
    bridge_c.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: None,
        via_profile_ids: vec![bob_id.clone()],
    })?;
    wait_for_event(&bridge_c, "Carol re-admitted under Bob", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Member)
            .map(|_| ())
    })
    .await?;
    bridge_b.send(group_post(&group_id, "the river keeps moving"))?;
    wait_for_event(&bridge_c, "post-transfer post at Carol", |event| {
        group_view_of(event, &group_id)
            .filter(|view| {
                view.posts
                    .iter()
                    .any(|post| post.body == "the river keeps moving")
            })
            .map(|_| ())
    })
    .await?;

    Ok(())
}

/// Request-to-join with an offline Owner: the request stays pending until
/// the Owner's node processes it (ADR-0005), pending requests are visible
/// only to the Owner, and the requester sees their own pending state.
#[tokio::test(flavor = "multi_thread")]
async fn request_to_join_waits_for_the_owner() -> Result<()> {
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
    let (bridge_a, alice_id) = spawn_onboarded(dir_a.path(), node_options.clone(), "Alice").await?;
    let (bridge_b, bob_id) = spawn_onboarded(dir_b.path(), node_options.clone(), "Bob").await?;

    // Alice creates a request-to-join group; Bob syncs it while she is
    // online so the topic has a second seeder.
    bridge_a.send(NetworkCommand::CreateGroup {
        name: "ask first".into(),
        content_mode: GroupContentMode::Public,
        join_mode: GroupJoinMode::Request,
        discoverability: GroupDiscoverability::Listed,
    })?;
    let group_id = wait_for_event(&bridge_a, "group creation", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.name == "ask first" => {
            Some(view.group_id.clone())
        }
        _ => None,
    })
    .await?;
    bridge_b.send(NetworkCommand::SyncGroup {
        group_id: group_id.clone(),
        via_profile_ids: vec![alice_id.clone()],
    })?;
    wait_for_event(&bridge_b, "Bob to see the group", |event| {
        group_view_of(event, &group_id).map(|_| ())
    })
    .await?;

    // Alice goes offline; Bob requests to join into the void.
    drop(bridge_a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    bridge_b.send(NetworkCommand::JoinGroup {
        group_id: group_id.clone(),
        greeting: Some("open the door?".into()),
        via_profile_ids: vec![alice_id.clone()],
    })?;
    // The requester sees their own pending state — and, not being the
    // Owner, no pending-requests list.
    let bob_pending = wait_for_event(&bridge_b, "Bob's own pending state", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Pending)
            .cloned()
    })
    .await?;
    assert!(bob_pending.pending_requests.is_empty());

    // Alice returns; her node picks the request up from the topic and, in
    // Request mode, surfaces it instead of auto-accepting.
    let bridge_a = AsyncBridge::spawn_with_data_dir(node_options, dir_a.path().to_path_buf())?;
    let alice_view = wait_for_event(&bridge_a, "the pending request at Alice", |event| {
        group_view_of(event, &group_id)
            .filter(|view| !view.pending_requests.is_empty())
            .cloned()
    })
    .await?;
    assert_eq!(alice_view.pending_requests[0].requester_profile_id, bob_id);
    assert_eq!(
        alice_view.pending_requests[0].greeting.as_deref(),
        Some("open the door?")
    );
    // Still not a member: request mode never auto-accepts.
    assert!(!alice_view.members.iter().any(|m| m.profile_id == bob_id));

    // Alice approves; the join completes for Bob.
    bridge_a.send(NetworkCommand::RespondGroupJoin {
        group_id: group_id.clone(),
        requester_profile_id: bob_id.clone(),
        accept: true,
    })?;
    wait_for_event(&bridge_b, "Bob to become a member", |event| {
        group_view_of(event, &group_id)
            .filter(|view| view.viewer_status == GroupViewerStatus::Member)
            .map(|_| ())
    })
    .await?;

    Ok(())
}
