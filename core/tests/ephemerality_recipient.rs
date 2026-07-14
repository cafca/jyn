//! Phase-3 ephemerality, recipient side: when a non-public post I received
//! from a friend (or friend-of-friend) expires, it is torn down on MY device
//! too — its media cache is pruned, its ciphertext is no longer served, and its
//! decrypted text is purged — so expired content leaves the whole network, not
//! just the author. A copy I explicitly kept survives, and an offline recipient
//! catches up on next start. Driven at the `AsyncBridge` seam over a real relay,
//! mirroring the friendship / circles integration suites.
//!
//! Expiry is deterministic: the author gives the post an `expires_at` already
//! in the past, then the recipient's drain is triggered explicitly. Teardown is
//! asserted on state the recipient synchronously controls — the plaintext cache
//! file and the recipient's reduced view — never on the store's async GC.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, MediaDraft, NetworkCommand, NetworkEvent, PostDraft};
use jyn::domain::{MediaKind, Visibility};
use jyn::friend_code::FriendCode;
use jyn::node::NodeOptions;
use tempfile::{tempdir, TempDir};

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

fn node_options(relay_url: &iroh::RelayUrl) -> NodeOptions {
    NodeOptions {
        relay_url: Some(relay_url.clone()),
        mdns_enabled: false,
        insecure_skip_relay_cert_verify: true,
        gc_enabled: false,
    }
}

async fn spawn_onboarded(
    data_dir: &Path,
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

/// Runs the full request → accept → follow-back handshake until mutual.
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

fn cache_file(data_dir: &Path, blob_hash: &str) -> PathBuf {
    data_dir.join("media-cache").join(blob_hash)
}

async fn drain(bridge: &AsyncBridge) -> Result<()> {
    bridge
        .send_awaited(NetworkCommand::DrainExpired)?
        .await
        .context("drain command dropped")?
        .map_err(|err| anyhow::anyhow!(err))
}

async fn recover(bridge: &AsyncBridge) -> Result<()> {
    bridge
        .send_awaited(NetworkCommand::RecoverStartup)?
        .await
        .context("recover command dropped")?
        .map_err(|err| anyhow::anyhow!(err))
}

/// Publishes a post carrying one attachment and returns (post_id, blob_hash)
/// once it surfaces in the author's own local state.
async fn publish_with_media(
    bridge: &AsyncBridge,
    staging: &TempDir,
    name: &str,
    body: &str,
    visibility: Visibility,
) -> Result<(String, String)> {
    let path = staging.path().join(name);
    std::fs::write(&path, format!("secret bytes for {name}").as_bytes())?;
    bridge.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: body.into(),
            visibility,
            lifetime_secs: None,
            media: vec![MediaDraft {
                path,
                kind: MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;
    wait_for_event(bridge, "own post with attachment", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => state
            .posts
            .iter()
            .find(|post| post.body == body && !post.media.is_empty())
            .map(|post| (post.post_id.clone(), post.media[0].blob_hash.clone())),
        _ => None,
    })
    .await
}

/// Waits until `contact`'s reduced view of `author` contains a post with
/// `body`, returning that post's attachment blob hash.
async fn wait_for_received(recipient: &AsyncBridge, author_id: &str, body: &str) -> Result<String> {
    wait_for_event(recipient, "received post", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if profile_id == author_id => state
            .posts
            .iter()
            .find(|post| post.body == body && !post.media.is_empty())
            .map(|post| post.media[0].blob_hash.clone()),
        _ => None,
    })
    .await
}

async fn fetch_media(recipient: &AsyncBridge, blob_hash: &str) -> Result<PathBuf> {
    recipient.send(NetworkCommand::FetchMedia {
        blob_hash: blob_hash.to_owned(),
    })?;
    wait_for_event(recipient, "media fetch", |event| match event {
        NetworkEvent::MediaReady {
            blob_hash: ready,
            path,
        } if ready == blob_hash => Some(path.clone()),
        NetworkEvent::MediaFailed {
            blob_hash: failed,
            error_message,
        } if failed == blob_hash => {
            // Surface as an error by returning a sentinel the caller asserts on.
            panic!("media fetch failed for {failed}: {error_message}")
        }
        _ => None,
    })
    .await
}

/// Sets a post's lifetime to a past instant on the author and waits until the
/// recipient's reduced view of the author reflects the expiry.
async fn expire_on_author_and_await_recipient(
    author: &AsyncBridge,
    recipient: &AsyncBridge,
    author_id: &str,
    post_id: &str,
) -> Result<()> {
    author.send(NetworkCommand::SetPostLifetime {
        post_id: post_id.to_owned(),
        expires_at: Some(1),
    })?;
    wait_for_event(
        recipient,
        "recipient to see the expiry",
        |event| match event {
            NetworkEvent::ContactStateUpdated { profile_id, state } if profile_id == author_id => {
                state
                    .posts
                    .iter()
                    .find(|post| post.post_id == post_id)
                    .and_then(|post| (post.expires_at == Some(1)).then_some(()))
            }
            _ => None,
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_post_is_torn_down_on_the_recipient() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3600");
    let (_relay_map, relay_url, _relay_server) = run_relay_server().await?;
    let opts = node_options(&relay_url);

    let dir_a = tempdir()?;
    let dir_b = tempdir()?;
    let staging = tempdir()?;
    let (alice, alice_id) = spawn_onboarded(dir_a.path(), opts.clone(), "Alice").await?;
    let (bob, bob_id) = spawn_onboarded(dir_b.path(), opts.clone(), "Bob").await?;
    befriend(&bob, &bob_id, &alice, &alice_id, relay_url.as_ref()).await?;

    // Alice posts to friends; Bob receives it and fetches the media.
    let (post_id, blob_hash) = publish_with_media(
        &alice,
        &staging,
        "shared.png",
        "for my friends",
        Visibility::Friends,
    )
    .await?;
    let received_hash = wait_for_received(&bob, &alice_id, "for my friends").await?;
    assert_eq!(received_hash, blob_hash);
    fetch_media(&bob, &blob_hash).await?;
    let bob_cache = cache_file(dir_b.path(), &blob_hash);
    assert!(
        bob_cache.is_file(),
        "Bob materialized the plaintext locally"
    );

    // Alice lets it expire; Bob learns the new lifetime, then drains.
    expire_on_author_and_await_recipient(&alice, &bob, &alice_id, &post_id).await?;
    drain(&bob).await?;

    // Bob's plaintext media cache is pruned...
    assert!(
        !bob_cache.exists(),
        "an expired received post's media cache is pruned on the recipient"
    );

    // ...and its decrypted text is purged, so the post leaves Bob's reduced
    // view of Alice entirely (not merely filtered as expired). Replay startup
    // recovery to re-emit Alice's reduced state and confirm.
    recover(&bob).await?;
    let alice_view = wait_for_event(&bob, "Bob's refreshed view of Alice", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == alice_id => {
            Some(state.clone())
        }
        _ => None,
    })
    .await?;
    assert!(
        alice_view.posts.iter().all(|post| post.post_id != post_id),
        "the decrypted text is purged, so the expired post cannot be reconstructed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recipients_kept_copy_survives_the_originals_expiry() -> Result<()> {
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3600");
    let (_relay_map, relay_url, _relay_server) = run_relay_server().await?;
    let opts = node_options(&relay_url);

    let dir_a = tempdir()?;
    let dir_b = tempdir()?;
    let staging = tempdir()?;
    let (alice, alice_id) = spawn_onboarded(dir_a.path(), opts.clone(), "Alice").await?;
    let (bob, bob_id) = spawn_onboarded(dir_b.path(), opts.clone(), "Bob").await?;
    befriend(&bob, &bob_id, &alice, &alice_id, relay_url.as_ref()).await?;

    let (post_id, blob_hash) = publish_with_media(
        &alice,
        &staging,
        "cherished.png",
        "hold this for me",
        Visibility::Friends,
    )
    .await?;
    wait_for_received(&bob, &alice_id, "hold this for me").await?;
    fetch_media(&bob, &blob_hash).await?;

    // Bob keeps Alice's post: a lease under his own keep/ pin namespace.
    bob.send(NetworkCommand::KeepPost {
        post_author_profile_id: alice_id.clone(),
        post_id: post_id.clone(),
    })?;
    wait_for_event(&bob, "keep recorded", |event| match event {
        NetworkEvent::KeepsUpdated { keeps } => keeps
            .iter()
            .any(|keep| keep.post_id == post_id)
            .then_some(()),
        _ => None,
    })
    .await?;

    // Alice expires it; Bob drains. His feed-side view is torn down, but the
    // keep must survive — its blob is held under keep/ and its text lives in
    // the keep snapshot.
    expire_on_author_and_await_recipient(&alice, &bob, &alice_id, &post_id).await?;
    drain(&bob).await?;

    // The keep record survives startup recovery...
    recover(&bob).await?;
    let kept = wait_for_event(&bob, "keep survived", |event| match event {
        NetworkEvent::KeepsUpdated { keeps } => {
            keeps.iter().find(|keep| keep.post_id == post_id).cloned()
        }
        _ => None,
    })
    .await?;
    assert_eq!(
        kept.snapshot.media.first().map(|m| m.blob_hash.as_str()),
        Some(blob_hash.as_str()),
        "the kept copy retains its media reference"
    );

    // ...and the kept media is still recoverable locally: it re-exports from
    // the store because the keep/ pin still holds the blob, and its secret
    // comes from the keep snapshot.
    let path = fetch_media(&bob, &blob_hash).await?;
    assert!(
        path.is_file(),
        "the kept copy's media survives the teardown"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_recipient_tears_down_on_next_start() -> Result<()> {
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3600");
    let (_relay_map, relay_url, _relay_server) = run_relay_server().await?;
    let opts = node_options(&relay_url);

    let dir_a = tempdir()?;
    let dir_b = tempdir()?;
    let staging = tempdir()?;
    let (alice, alice_id) = spawn_onboarded(dir_a.path(), opts.clone(), "Alice").await?;
    let (bob, bob_id) = spawn_onboarded(dir_b.path(), opts.clone(), "Bob").await?;
    befriend(&bob, &bob_id, &alice, &alice_id, relay_url.as_ref()).await?;

    let (post_id, blob_hash) = publish_with_media(
        &alice,
        &staging,
        "fading.png",
        "gone by morning",
        Visibility::Friends,
    )
    .await?;
    wait_for_received(&bob, &alice_id, "gone by morning").await?;
    fetch_media(&bob, &blob_hash).await?;
    let bob_cache = cache_file(dir_b.path(), &blob_hash);
    assert!(bob_cache.is_file());

    // Bob learns of the expiry but does NOT drain — he was "offline" at the
    // moment it expired.
    expire_on_author_and_await_recipient(&alice, &bob, &alice_id, &post_id).await?;
    assert!(bob_cache.is_file(), "nothing tore it down at expiry time");

    // On next start, Bob's startup recovery runs the same teardown and catches
    // up. Idempotent: a second recovery is a no-op.
    recover(&bob).await?;
    assert!(
        !bob_cache.exists(),
        "startup recovery tears down an expired received post"
    );
    recover(&bob).await?;
    assert!(!bob_cache.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_friend_of_friend_tears_down_an_expired_circles_post() -> Result<()> {
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3600");
    let (_relay_map, relay_url, _relay_server) = run_relay_server().await?;
    let opts = node_options(&relay_url);

    // Alice–Bob and Bob–Carol are friends, so Carol is in Alice's circle
    // (friends-of-friends) but not a direct friend.
    let dir_a = tempdir()?;
    let dir_b = tempdir()?;
    let dir_c = tempdir()?;
    let staging = tempdir()?;
    let (alice, alice_id) = spawn_onboarded(dir_a.path(), opts.clone(), "Alice").await?;
    let (bob, bob_id) = spawn_onboarded(dir_b.path(), opts.clone(), "Bob").await?;
    let (carol, carol_id) = spawn_onboarded(dir_c.path(), opts.clone(), "Carol").await?;
    befriend(&bob, &bob_id, &alice, &alice_id, relay_url.as_ref()).await?;
    befriend(&carol, &carol_id, &bob, &bob_id, relay_url.as_ref()).await?;
    alice.send(NetworkCommand::ReconcileSpaces)?;
    carol.send(NetworkCommand::ReconcileSpaces)?;

    // Alice casts a Circles post with media. Carol's admission (her key bundle
    // must travel first) can race the post, so retry with a fresh post until
    // one decrypts for Carol, then capture exactly that post.
    let photo = staging.path().join("circle.png");
    let photo_bytes = b"ripple bytes for the second ring".to_vec();
    std::fs::write(&photo, &photo_bytes)?;
    let (carol_post_id, blob_hash) = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            alice.send(NetworkCommand::PublishPost {
                draft: PostDraft {
                    body: "ripples reach the second ring".into(),
                    visibility: Visibility::Circles,
                    lifetime_secs: None,
                    media: vec![MediaDraft {
                        path: photo.clone(),
                        kind: MediaKind::Photo,
                        duration_ms: None,
                        waveform: None,
                    }],
                },
            })?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while tokio::time::Instant::now() < deadline {
                for event in carol.drain_events() {
                    if let NetworkEvent::ContactStateUpdated { profile_id, state } = event {
                        if profile_id == alice_id {
                            if let Some(post) = state
                                .posts
                                .iter()
                                .find(|post| post.body == "ripples reach the second ring")
                                .filter(|post| !post.media.is_empty())
                            {
                                return anyhow::Ok((
                                    post.post_id.clone(),
                                    post.media[0].blob_hash.clone(),
                                ));
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    })
    .await
    .context("timed out waiting for Alice's circles post to reach Carol")??;

    // Carol (a friend-of-friend) fetches the media. It must decrypt to the
    // original plaintext — a FoF's per-blob secret lives in the circle author's
    // reduced state, not in Carol's direct-friend list.
    let fetched = fetch_media(&carol, &blob_hash).await?;
    assert_eq!(
        std::fs::read(&fetched)?,
        photo_bytes,
        "a friend-of-friend decrypts the circles media rather than caching ciphertext"
    );
    let carol_cache = cache_file(dir_c.path(), &blob_hash);
    assert!(
        carol_cache.is_file(),
        "Carol materialized the circles media"
    );

    // Alice lets that post expire; Carol learns of it and drains.
    expire_on_author_and_await_recipient(&alice, &carol, &alice_id, &carol_post_id).await?;
    drain(&carol).await?;

    assert!(
        !carol_cache.exists(),
        "a friend-of-friend tears down an expired circles post's media"
    );
    Ok(())
}
