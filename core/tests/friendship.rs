//! End-to-end friendship flow over a real relay, driven through the same
//! bridge the UI uses: request → accept → follow-back → posts flow both
//! ways — including the case where the request's target was offline.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, MediaDraft, NetworkCommand, NetworkEvent, PostDraft};
use jyn::domain::Visibility;
use jyn::friend_code::FriendCode;
use jyn::node::NodeOptions;
use tempfile::tempdir;

const EVENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Polls a bridge's event stream until `select` yields a value.
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

/// Spawns a bridge on a fresh data dir and onboards it under `name`.
/// Returns the bridge and the profile id.
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

fn friend_code_for(profile_id: &str, relay_url: &str, name: &str) -> Result<String> {
    let key = profile_id.parse()?;
    FriendCode::new(key, Some(relay_url.to_owned()), name).encode()
}

#[tokio::test(flavor = "multi_thread")]
async fn request_accept_and_posts_flow_both_ways() -> Result<()> {
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
    let (bridge_a, alice_id) = spawn_onboarded(dir_a.path(), node_options.clone(), "Alice").await?;
    let (bridge_b, bob_id) = spawn_onboarded(dir_b.path(), node_options.clone(), "Bob").await?;

    // Bob enters Alice's code: the request lands in Alice's pending list.
    let code = friend_code_for(&alice_id, relay_url.as_ref(), "Alice")?;
    bridge_b.send(NetworkCommand::RequestFriendship {
        friend_code: code,
        greeting: Some("river sent me".into()),
    })?;
    wait_for_event(
        &bridge_a,
        "Alice to see Bob's request",
        |event| match event {
            NetworkEvent::LocalStateUpdated { state } => state
                .pending_requests
                .iter()
                .any(|request| request.requester_profile_id == bob_id)
                .then_some(()),
            _ => None,
        },
    )
    .await?;

    // Alice accepts; Bob observes her follow-back and completes the
    // friendship (the UI does this automatically via FollowBack).
    bridge_a.send(NetworkCommand::RespondFriendship {
        requester_profile_id: bob_id.clone(),
        accept: true,
    })?;
    wait_for_event(
        &bridge_b,
        "Bob to observe acceptance",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => {
                state.followed_profile_ids.contains(&bob_id).then_some(())
            }
            _ => None,
        },
    )
    .await?;
    bridge_b.send(NetworkCommand::FollowBack {
        profile_id: alice_id.clone(),
    })?;
    wait_for_event(
        &bridge_a,
        "friendship to become mutual",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == bob_id => {
                state.followed_profile_ids.contains(&alice_id).then_some(())
            }
            _ => None,
        },
    )
    .await?;

    // Posts flow in both directions.
    bridge_a.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "first light on the water".into(),
            visibility: Visibility::Friends,
            lifetime_secs: None,
            media: Vec::new(),
        },
    })?;
    wait_for_event(
        &bridge_b,
        "Alice's post to reach Bob",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => {
                state
                    .posts
                    .iter()
                    .any(|post| post.body == "first light on the water")
                    .then_some(())
            }
            _ => None,
        },
    )
    .await?;

    bridge_b.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "casting back".into(),
            visibility: Visibility::Friends,
            lifetime_secs: Some(3600),
            media: Vec::new(),
        },
    })?;
    wait_for_event(
        &bridge_a,
        "Bob's post to reach Alice",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == bob_id => {
                state
                    .posts
                    .iter()
                    .any(|post| post.body == "casting back")
                    .then_some(())
            }
            _ => None,
        },
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn request_reaches_target_who_was_offline() -> Result<()> {
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

    // Alice exists (identity created, onboarded) but goes offline.
    let dir_a = tempdir()?;
    let alice_id = {
        let (bridge_a, alice_id) =
            spawn_onboarded(dir_a.path(), node_options.clone(), "Alice").await?;
        drop(bridge_a);
        alice_id
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Bob requests friendship while Alice is offline.
    let dir_b = tempdir()?;
    let (bridge_b, bob_id) = spawn_onboarded(dir_b.path(), node_options.clone(), "Bob").await?;
    let code = friend_code_for(&alice_id, relay_url.as_ref(), "Alice")?;
    bridge_b.send(NetworkCommand::RequestFriendship {
        friend_code: code,
        greeting: None,
    })?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Alice comes back online; Bob's periodic maintenance re-initiates the
    // session and the request reaches her.
    let bridge_a = AsyncBridge::spawn_with_data_dir(node_options, dir_a.path().to_path_buf())?;
    wait_for_event(
        &bridge_a,
        "offline-published request to reach Alice",
        |event| match event {
            NetworkEvent::LocalStateUpdated { state } => state
                .pending_requests
                .iter()
                .any(|request| request.requester_profile_id == bob_id)
                .then_some(()),
            _ => None,
        },
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn media_attachments_round_trip_between_friends() -> Result<()> {
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

    // Fast-path friendship (covered in detail by the other test).
    let code = friend_code_for(&alice_id, relay_url.as_ref(), "Alice")?;
    bridge_b.send(NetworkCommand::RequestFriendship {
        friend_code: code,
        greeting: None,
    })?;
    wait_for_event(&bridge_a, "request", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => {
            (!state.pending_requests.is_empty()).then_some(())
        }
        _ => None,
    })
    .await?;
    bridge_a.send(NetworkCommand::RespondFriendship {
        requester_profile_id: bob_id.clone(),
        accept: true,
    })?;
    wait_for_event(&bridge_b, "acceptance", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => {
            state.followed_profile_ids.contains(&bob_id).then_some(())
        }
        _ => None,
    })
    .await?;
    bridge_b.send(NetworkCommand::FollowBack {
        profile_id: alice_id.clone(),
    })?;
    wait_for_event(&bridge_a, "mutuality", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == bob_id => {
            state.followed_profile_ids.contains(&alice_id).then_some(())
        }
        _ => None,
    })
    .await?;

    // Alice casts a post with an attached "photo"; Bob's runtime fetches the
    // blob and lands it in his media cache, byte-identical.
    let photo_path = dir_a.path().join("sunrise.png");
    let photo_bytes = b"not-really-a-png-but-bytes-are-bytes".to_vec();
    std::fs::write(&photo_path, &photo_bytes)?;
    bridge_a.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "first light".into(),
            visibility: jyn::domain::Visibility::Friends,
            lifetime_secs: None,
            media: vec![MediaDraft {
                path: photo_path,
                kind: jyn::domain::MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;

    let blob_hash = wait_for_event(&bridge_b, "post with attachment", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => state
            .posts
            .iter()
            .find(|post| !post.media.is_empty())
            .map(|post| post.media[0].blob_hash.clone()),
        _ => None,
    })
    .await?;

    bridge_b.send(NetworkCommand::FetchMedia {
        blob_hash: blob_hash.clone(),
    })?;
    let fetched_path = wait_for_event(&bridge_b, "media to arrive", |event| match event {
        NetworkEvent::MediaReady {
            blob_hash: ready_hash,
            path,
        } if *ready_hash == blob_hash => Some(path.clone()),
        _ => None,
    })
    .await?;

    assert_eq!(std::fs::read(fetched_path)?, photo_bytes);
    Ok(())
}
