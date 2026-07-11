//! The spec's phase-2 recovery promise: a backup carries blob bytes for
//! live and kept posts, so a restore onto a fresh machine serves media with
//! zero dependence on peers being online.

use std::time::Duration;

use anyhow::{Context, Result};
use jyn::bridge::{AsyncBridge, MediaDraft, NetworkCommand, NetworkEvent, PostDraft};
use jyn::domain::Visibility;
use jyn::node::NodeOptions;
use tempfile::tempdir;

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

#[tokio::test(flavor = "multi_thread")]
async fn backup_carries_blobs_and_restore_serves_media_without_peers() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    std::env::set_var("JYN_MAINTENANCE_INTERVAL_SECS", "3600");
    let node_options = NodeOptions {
        relay_url: None,
        mdns_enabled: false,
        insecure_skip_relay_cert_verify: false,
        // Off so nodes can reopen the same data dir in-process (GC keeps the
        // blob store resident).
        gc_enabled: false,
    };

    let dir = tempdir()?;
    let bridge = AsyncBridge::spawn_with_data_dir(node_options.clone(), dir.path().to_path_buf())?;
    wait_for_event(&bridge, "ProfileLoaded", |event| match event {
        NetworkEvent::ProfileLoaded { .. } => Some(()),
        _ => None,
    })
    .await?;

    // A Friends post with a sealed attachment: the blob store holds
    // ciphertext, the backup must carry those exact bytes.
    let photo_path = dir.path().join("sunrise.png");
    let photo_bytes = b"the-original-plaintext-photo-bytes".to_vec();
    std::fs::write(&photo_path, &photo_bytes)?;
    bridge.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "first light".into(),
            visibility: Visibility::Friends,
            lifetime_secs: None,
            media: vec![MediaDraft {
                path: photo_path,
                kind: jyn::domain::MediaKind::Photo,
                duration_ms: None,
                waveform: None,
            }],
        },
    })?;
    let blob_hash = wait_for_event(&bridge, "post with attachment", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => state
            .posts
            .iter()
            .find(|post| !post.media.is_empty())
            .map(|post| post.media[0].blob_hash.clone()),
        _ => None,
    })
    .await?;

    let phrase = {
        let private_key = jyn::profile::load_private_key_from_data_dir(dir.path())?;
        jyn::backup::seed_phrase(&private_key)?
    };
    let archive = dir.path().join("jyn.backup");
    let receiver = bridge.send_awaited(NetworkCommand::ExportBackup {
        dest_path: archive.to_string_lossy().into_owned(),
    })?;
    receiver
        .await
        .context("export command dropped")?
        .map_err(|err| anyhow::anyhow!(err))?;

    // The profile-data store must stay usable after the export's VACUUM
    // snapshot — a private post exercises the same pool.
    bridge.send(NetworkCommand::PublishPost {
        draft: PostDraft {
            body: "post-export private note".into(),
            visibility: Visibility::Private,
            lifetime_secs: None,
            media: Vec::new(),
        },
    })?;
    wait_for_event(&bridge, "private post after export", |event| match event {
        NetworkEvent::PrivatePostsUpdated { posts } => posts
            .iter()
            .any(|post| post.body == "post-export private note")
            .then_some(()),
        _ => None,
    })
    .await?;
    drop(bridge);

    // Restore onto a "new machine". No relay, no peers: the media can only
    // come from the archive.
    let restored_dir = tempdir()?;
    let restored_data = restored_dir.path().join("data");
    jyn::backup::restore_backup(&restored_data, &archive, &phrase)?;
    assert!(
        restored_data
            .join(jyn::backup::RESTORED_BLOBS_DIR)
            .join(&blob_hash)
            .is_file(),
        "restore stages the blob bytes for import"
    );

    let bridge = AsyncBridge::spawn_with_data_dir(node_options, restored_data.clone())?;
    wait_for_event(&bridge, "restored post", |event| match event {
        NetworkEvent::LocalStateUpdated { state } => state
            .posts
            .iter()
            .any(|post| post.body == "first light")
            .then_some(()),
        _ => None,
    })
    .await?;

    bridge.send(NetworkCommand::FetchMedia {
        blob_hash: blob_hash.clone(),
    })?;
    let fetched_path = wait_for_event(&bridge, "restored media", |event| match event {
        NetworkEvent::MediaReady {
            blob_hash: ready_hash,
            path,
        } if *ready_hash == blob_hash => Some(path.clone()),
        _ => None,
    })
    .await?;
    assert_eq!(
        std::fs::read(fetched_path)?,
        photo_bytes,
        "restored blob decrypts back to the original media"
    );

    // The staging dir is consumed by the import.
    assert!(!restored_data.join(jyn::backup::RESTORED_BLOBS_DIR).exists());

    Ok(())
}
