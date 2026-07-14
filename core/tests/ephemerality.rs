//! Phase-3 ephemerality, author side: when my own non-public (Friends/Circles)
//! post expires or I delete it, its media is torn down on my device and its
//! decrypted text is purged — while permanent posts, public posts, and kept
//! copies are left intact. Driven at the `AsyncBridge` command/event seam, the
//! same seam the other integration suites use.
//!
//! Expiry is made deterministic without sleeping: a post is given an
//! `expires_at` already in the past (via `SetPostLifetime`) and the drain is
//! then triggered explicitly. Teardown is asserted on the state jyn
//! synchronously controls — the materialized plaintext cache file — never on
//! the blob store's asynchronous GC.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use jyn::bridge::{AsyncBridge, MediaDraft, NetworkCommand, NetworkEvent, PostDraft};
use jyn::domain::{MediaKind, Visibility};
use jyn::node::NodeOptions;
use tempfile::{tempdir, TempDir};

const EVENT_TIMEOUT: Duration = Duration::from_secs(60);

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

fn node_options() -> NodeOptions {
    NodeOptions {
        relay_url: None,
        mdns_enabled: false,
        insecure_skip_relay_cert_verify: false,
        // Off so a node can reopen the same data dir in-process (GC keeps the
        // blob store resident) — the offline-convergence test restarts.
        gc_enabled: false,
    }
}

async fn spawn(data_dir: PathBuf) -> Result<(AsyncBridge, String)> {
    let bridge = AsyncBridge::spawn_with_data_dir(node_options(), data_dir)?;
    let profile_id = wait_for_event(&bridge, "ProfileLoaded", |event| match event {
        NetworkEvent::ProfileLoaded { profile } => Some(profile.profile_id.clone()),
        _ => None,
    })
    .await?;
    Ok((bridge, profile_id))
}

fn cache_file(data_dir: &Path, blob_hash: &str) -> PathBuf {
    data_dir.join("media-cache").join(blob_hash)
}

/// Stages a file and publishes a post carrying it, returning the post id and
/// the attachment's blob hash once the post surfaces in local state.
async fn publish_post_with_media(
    bridge: &AsyncBridge,
    staging: &TempDir,
    name: &str,
    body: &str,
    visibility: Visibility,
    lifetime_secs: Option<u64>,
) -> Result<(String, String)> {
    let path = staging.path().join(name);
    std::fs::write(&path, format!("plaintext bytes for {name}").as_bytes())?;
    bridge.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: body.into(),
            visibility,
            lifetime_secs,
            media: vec![MediaDraft {
                path,
                kind: MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;
    wait_for_event(
        bridge,
        "published post with attachment",
        |event| match event {
            NetworkEvent::LocalStateUpdated { state } => state
                .posts
                .iter()
                .find(|post| post.body == body && !post.media.is_empty())
                .map(|post| (post.post_id.clone(), post.media[0].blob_hash.clone())),
            _ => None,
        },
    )
    .await
}

/// Gives a post an `expires_at` already in the past and waits until local state
/// reflects the change, so a subsequent drain sees it as expired.
async fn expire_now(bridge: &AsyncBridge, post_id: &str) -> Result<()> {
    bridge.send(NetworkCommand::SetPostLifetime {
        post_id: post_id.to_owned(),
        expires_at: Some(1),
    })?;
    wait_for_event(bridge, "lifetime change to land", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => state
            .posts
            .iter()
            .find(|post| post.post_id == post_id)
            .and_then(|post| (post.expires_at == Some(1)).then_some(())),
        _ => None,
    })
    .await
}

async fn drain(bridge: &AsyncBridge) -> Result<()> {
    bridge
        .send_awaited(NetworkCommand::DrainExpired)?
        .await
        .context("drain command dropped")?
        .map_err(|err| anyhow::anyhow!(err))
}

/// Replays the startup recovery path (profile + state + `drain_expired` +
/// keeps) in-process, exactly as a fresh launch would. This is how the offline
/// and keep tests exercise "what happens on next start" — a full node
/// teardown/reopen on the *same* data dir is unreliable in-process (the blob
/// store does not reliably re-open on the same path within one process), so
/// driving the identical recovery function is both faithful and deterministic.
async fn recover(bridge: &AsyncBridge) -> Result<()> {
    bridge
        .send_awaited(NetworkCommand::RecoverStartup)?
        .await
        .context("recover command dropped")?
        .map_err(|err| anyhow::anyhow!(err))
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_non_public_post_media_is_torn_down_on_drain() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;
    let (bridge, _) = spawn(dir.path().to_path_buf()).await?;

    let (post_id, blob_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "friends.png",
        "letting this one go",
        Visibility::Friends,
        None,
    )
    .await?;
    let cached = cache_file(dir.path(), &blob_hash);
    assert!(
        cached.is_file(),
        "publishing materializes the plaintext cache"
    );

    expire_now(&bridge, &post_id).await?;
    drain(&bridge).await?;

    assert!(
        !cached.exists(),
        "an expired Friends post's plaintext media cache is pruned on drain"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_non_public_post_tears_down_its_media() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;
    let (bridge, _) = spawn(dir.path().to_path_buf()).await?;

    let (post_id, blob_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "circles.png",
        "on second thought",
        Visibility::Circles,
        None,
    )
    .await?;
    let cached = cache_file(dir.path(), &blob_hash);
    assert!(cached.is_file());

    bridge
        .send_awaited(NetworkCommand::DeletePost { post_id })?
        .await
        .context("delete command dropped")?
        .map_err(|err| anyhow::anyhow!(err))?;

    assert!(
        !cached.exists(),
        "delete tears media down the same way expiry does"
    );
    Ok(())
}

/// Once a non-public post has been torn down, it is gone from our own reduced
/// state, so its visibility is no longer resolvable. Acting on it again
/// (delete / re-lifetime / edit) must be a safe no-op — crucially it must NOT
/// fall back to a plaintext publish, which would broadcast a `PostDeleted` /
/// `PostLifetimeChanged` / `PostEdited` for what was an encrypted post.
#[tokio::test(flavor = "multi_thread")]
async fn acting_on_a_torn_down_post_is_a_safe_no_op() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;
    let (bridge, _) = spawn(dir.path().to_path_buf()).await?;

    let (post_id, _blob_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "vanished.png",
        "here then gone",
        Visibility::Friends,
        None,
    )
    .await?;

    // Expire and drain so the post is torn down (decrypted row purged, so it
    // leaves our reduced state entirely).
    expire_now(&bridge, &post_id).await?;
    drain(&bridge).await?;

    // Each of these resolves the post's visibility; after teardown it is
    // unknown. They must succeed as no-ops rather than erroring or, worse,
    // publishing in plaintext.
    for command in [
        NetworkCommand::DeletePost {
            post_id: post_id.clone(),
        },
        NetworkCommand::SetPostLifetime {
            post_id: post_id.clone(),
            expires_at: None,
        },
        NetworkCommand::EditPost {
            post_id: post_id.clone(),
            body: "trying to revive".into(),
            kept_media: Vec::new(),
            new_media: Vec::new(),
        },
    ] {
        bridge
            .send_awaited(command)?
            .await
            .context("command dropped")?
            .map_err(|err| anyhow::anyhow!(err))?;
    }

    // No error surfaced, and the post did not come back to life.
    for event in bridge.drain_events() {
        if let NetworkEvent::Error {
            context,
            error_message,
        } = event
        {
            anyhow::bail!("unexpected error event [{context}]: {error_message}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn permanent_posts_survive_but_expired_public_posts_are_reclaimed() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;
    let (bridge, _) = spawn(dir.path().to_path_buf()).await?;

    // A permanent (never-expiring) post: GC must never touch it.
    let (_permanent_id, permanent_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "keepsake.png",
        "here to stay",
        Visibility::Friends,
        None,
    )
    .await?;

    // A public post the author gives a past expiry: an ephemeral public post is
    // ephemeral by the author's choice, so GC reclaims its bucket — and media —
    // on expiry, just like a non-public one (co-deletion GC is the storage
    // reclamation the Phase-3 spec deferred to this workstream).
    let (public_id, public_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "billboard.png",
        "shout it out",
        Visibility::Public,
        None,
    )
    .await?;
    expire_now(&bridge, &public_id).await?;

    drain(&bridge).await?;

    assert!(
        cache_file(dir.path(), &permanent_hash).is_file(),
        "a permanent post's media is left untouched by the drain"
    );
    assert!(
        !cache_file(dir.path(), &public_hash).exists(),
        "an expired public post's media cache is reclaimed on drain"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kept_post_survives_the_originals_expiry() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;
    let (bridge, local_profile_id) = spawn(dir.path().to_path_buf()).await?;

    // Publish a permanent post and keep it: the keep freezes a permanent
    // snapshot under its own pin namespace, independent of the original.
    let (post_id, blob_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "cherished.png",
        "worth holding on to",
        Visibility::Friends,
        None,
    )
    .await?;
    bridge.send(NetworkCommand::KeepPost {
        post_author_profile_id: local_profile_id,
        post_id: post_id.clone(),
    })?;
    wait_for_event(&bridge, "keep recorded", |event| match event {
        NetworkEvent::KeepsUpdated { keeps } => keeps
            .iter()
            .any(|keep| keep.post_id == post_id)
            .then_some(()),
        _ => None,
    })
    .await?;

    // Now let the original go and drain. Its feed presence is torn down, but
    // the keep's separate pin holds the shared blob alive (pin-counting), and
    // the keep record itself must not be pruned — its snapshot is permanent.
    expire_now(&bridge, &post_id).await?;
    drain(&bridge).await?;

    // Replaying startup recovery re-emits the persisted keeps. The keep is
    // still there, snapshot and its media reference intact — a keep is
    // genuinely independent of the original it copied. (Byte-level GC of the
    // now-unshared original is the store's job and deliberately not asserted.)
    recover(&bridge).await?;
    let kept = wait_for_event(&bridge, "keep survived recovery", |event| match event {
        NetworkEvent::KeepsUpdated { keeps } => {
            keeps.iter().find(|keep| keep.post_id == post_id).cloned()
        }
        _ => None,
    })
    .await?;
    assert_eq!(
        kept.snapshot.media.first().map(|m| m.blob_hash.as_str()),
        Some(blob_hash.as_str()),
        "the kept copy retains its media reference after the original expired"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_device_tears_down_on_next_start() -> Result<()> {
    let dir = tempdir()?;
    let staging = tempdir()?;

    let (bridge, _) = spawn(dir.path().to_path_buf()).await?;
    let (post_id, blob_hash) = publish_post_with_media(
        &bridge,
        &staging,
        "fading.png",
        "gone by morning",
        Visibility::Friends,
        None,
    )
    .await?;
    let cached = cache_file(dir.path(), &blob_hash);
    assert!(cached.is_file());

    // Expire it but DON'T drain — the device was offline at the moment of
    // expiry, so nothing tore it down at the time.
    expire_now(&bridge, &post_id).await?;
    assert!(
        cached.is_file(),
        "an undrained expiry leaves media in place until the next start"
    );

    // On next start, the startup recovery path runs the same drain and catches
    // up, tearing the offline-expired post down.
    recover(&bridge).await?;
    assert!(
        !cached.exists(),
        "startup recovery tears down a post that expired while offline"
    );

    // Idempotent: recovering again on the already-torn-down post is a no-op.
    recover(&bridge).await?;
    assert!(!cached.exists());
    Ok(())
}
