//! End-to-end circles flow over a real relay: Alice–Bob and Bob–Carol are
//! friends, which puts Carol in Alice's circle (friends-of-friends). Alice's
//! Circles posts reach Carol; her Friends posts never do. When Bob unfriends
//! Carol, the next Circles post re-keys her out (the spec's lazy re-key).

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, NetworkCommand, NetworkEvent, PostDraft};
use jyn::domain::Visibility;
use jyn::friend_code::FriendCode;
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

/// Runs the full request → accept → follow-back handshake between two
/// bridges until the friendship is mutual.
async fn befriend(
    requester: &AsyncBridge,
    requester_id: &str,
    target: &AsyncBridge,
    target_id: &str,
    relay_url: &str,
) -> Result<()> {
    let key = target_id.parse()?;
    let code = FriendCode::new(key, Some(relay_url.to_owned()), "friend").encode()?;
    requester.send(NetworkCommand::RequestFriendship {
        friend_code: code,
        greeting: None,
    })?;
    wait_for_event(target, "friendship request", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => state
            .pending_requests
            .iter()
            .any(|request| request.requester_profile_id == requester_id)
            .then_some(()),
        _ => None,
    })
    .await?;
    target.send(NetworkCommand::RespondFriendship {
        requester_profile_id: requester_id.to_owned(),
        accept: true,
    })?;
    wait_for_event(requester, "acceptance", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if profile_id == target_id => state
            .followed_profile_ids
            .iter()
            .any(|followed| followed == requester_id)
            .then_some(()),
        _ => None,
    })
    .await?;
    requester.send(NetworkCommand::FollowBack {
        profile_id: target_id.to_owned(),
    })?;
    wait_for_event(target, "mutuality", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if profile_id == requester_id => {
            state
                .followed_profile_ids
                .iter()
                .any(|followed| followed == target_id)
                .then_some(())
        }
        _ => None,
    })
    .await?;
    Ok(())
}

fn circles_post(body: &str) -> NetworkCommand {
    NetworkCommand::PublishPost {
        draft: PostDraft {
            body: body.into(),
            visibility: Visibility::Circles,
            lifetime_secs: None,
            media: Vec::new(),
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn circles_posts_reach_friends_of_friends_until_removed() -> Result<()> {
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

    befriend(&bridge_b, &bob_id, &bridge_a, &alice_id, relay_url.as_ref()).await?;
    befriend(&bridge_c, &carol_id, &bridge_b, &bob_id, relay_url.as_ref()).await?;

    // Alice and Carol are strangers with a mutual friend: each is in the
    // other's circle. Reconciling joins circle members' topics and adds them
    // to the circles space once their key bundles arrive (the bundle-arrival
    // path re-reconciles by itself afterwards).
    bridge_a.send(NetworkCommand::ReconcileSpaces)?;
    bridge_c.send(NetworkCommand::ReconcileSpaces)?;

    // A Friends post first: Carol must never see it, which the end of the
    // test can assert against the state that carried the circles posts.
    bridge_a.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "inner circle only".into(),
            visibility: Visibility::Friends,
            lifetime_secs: None,
            media: Vec::new(),
        },
    })?;
    wait_for_event(&bridge_b, "friends post at Bob", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => state
            .posts
            .iter()
            .any(|post| post.body == "inner circle only")
            .then_some(()),
        _ => None,
    })
    .await?;

    // The circles post may race Carol's admission (her key bundle has to
    // travel first), so retry until her copy of Alice's state shows it.
    let mut published = 0;
    let carol_view = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            bridge_a.send(circles_post("ripples reach the second ring"))?;
            published += 1;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while tokio::time::Instant::now() < deadline {
                for event in bridge_c.drain_events() {
                    if let NetworkEvent::ContactStateUpdated { profile_id, state } = event {
                        if profile_id == alice_id
                            && state
                                .posts
                                .iter()
                                .any(|post| post.body == "ripples reach the second ring")
                        {
                            return anyhow::Ok(state);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    })
    .await
    .context("timed out waiting for Alice's circles post to reach Carol")??;

    // Carol decrypted the circles post — and only that. The friends post is
    // invisible to her even though both traveled the same topic.
    assert!(
        carol_view
            .posts
            .iter()
            .all(|post| post.body != "inner circle only"),
        "a friends post must never decrypt for a friend-of-friend"
    );
    assert!(
        carol_view
            .posts
            .iter()
            .any(|post| post.visibility == Visibility::Circles),
        "the circles post keeps its visibility for readers"
    );

    // Bob sees it too: friends are part of the circle.
    wait_for_event(&bridge_b, "circles post at Bob", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => state
            .posts
            .iter()
            .any(|post| post.body == "ripples reach the second ring")
            .then_some(()),
        _ => None,
    })
    .await?;

    // Bob unfriends Carol: she leaves Alice's circle. The next circles post
    // lazily re-keys her out — Bob still reads it, Carol must not.
    bridge_b.send(NetworkCommand::RemoveFriend {
        profile_id: carol_id.clone(),
    })?;
    wait_for_event(
        &bridge_a,
        "Alice to see Bob drop Carol",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == bob_id => {
                state
                    .followed_profile_ids
                    .iter()
                    .all(|followed| followed != &carol_id)
                    .then_some(())
            }
            _ => None,
        },
    )
    .await?;

    bridge_a.send(circles_post("the ring tightens"))?;
    wait_for_event(
        &bridge_b,
        "post-removal circles post at Bob",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => {
                state
                    .posts
                    .iter()
                    .any(|post| post.body == "the ring tightens")
                    .then_some(())
            }
            _ => None,
        },
    )
    .await?;

    // Bob has it, so the wrapper is out on the topic; give Carol's node a
    // moment to ingest it, then assert it never decrypted for her.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let mut carol_sees_removed_post = false;
    for event in bridge_c.drain_events() {
        if let NetworkEvent::ContactStateUpdated { profile_id, state } = event {
            if profile_id == alice_id
                && state
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
        "a circles post published after removal must not decrypt for the removed member"
    );

    Ok(())
}
