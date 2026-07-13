//! Discovery + hearts over a real relay (ADR-0008, ADR-0009): friends learn
//! about `listed` groups through membership advertisements; hearts on
//! public+listed group posts surface outward as discovery-card data; an
//! `unlisted` group never surfaces by any mechanism.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use jyn::bridge::{AsyncBridge, GroupPostDraft, NetworkCommand, NetworkEvent};
use jyn::domain::Visibility;
use jyn::friend_code::FriendCode;
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

/// Listed groups surface to friends as hub suggestions; unlisted groups
/// never do. A heart on a public+listed group post reaches the hearter's
/// friends with the group context (the discovery-card data); a heart in an
/// unlisted group stays in-group.
#[tokio::test(flavor = "multi_thread")]
async fn listed_groups_suggest_and_hearts_propagate_outward() -> Result<()> {
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

    // Alice–Bob and Bob–Carol are friends; Alice and Carol are strangers.
    befriend(&bridge_b, &bob_id, &bridge_a, &alice_id, relay_url.as_ref()).await?;
    befriend(&bridge_c, &carol_id, &bridge_b, &bob_id, relay_url.as_ref()).await?;

    // Alice creates a listed and an unlisted group (both public + open).
    let create = |name: &str, discoverability| NetworkCommand::CreateGroup {
        name: name.to_owned(),
        content_mode: GroupContentMode::Public,
        join_mode: GroupJoinMode::Open,
        discoverability,
    };
    bridge_a.send(create("bird walks", GroupDiscoverability::Listed))?;
    let listed_group_id = wait_for_event(&bridge_a, "listed group creation", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.name == "bird walks" => {
            Some(view.group_id.clone())
        }
        _ => None,
    })
    .await?;
    bridge_a.send(create("secret picnic", GroupDiscoverability::Unlisted))?;
    let unlisted_group_id =
        wait_for_event(&bridge_a, "unlisted group creation", |event| match event {
            NetworkEvent::GroupUpdated { view } if view.name == "secret picnic" => {
                Some(view.group_id.clone())
            }
            _ => None,
        })
        .await?;

    // Bob's hub suggests the listed group via Alice — and the same snapshot
    // never carries the unlisted one (its membership is never advertised).
    let suggestion = wait_for_event(
        &bridge_b,
        "the listed group suggestion",
        |event| match event {
            NetworkEvent::GroupSuggestionsUpdated { suggestions } => {
                assert!(
                    suggestions
                        .iter()
                        .all(|suggestion| suggestion.group_id != unlisted_group_id),
                    "an unlisted group must never surface in suggestions"
                );
                suggestions
                    .iter()
                    .find(|suggestion| suggestion.group_id == listed_group_id)
                    .cloned()
            }
            _ => None,
        },
    )
    .await?;
    assert_eq!(suggestion.group_name, "bird walks");
    assert_eq!(suggestion.via_friend_profile_ids, vec![alice_id.clone()]);

    // Bob joins through the suggestion (the advertising friend seeds reach)
    // — and the group stops being a suggestion once he's a member.
    bridge_b.send(NetworkCommand::JoinGroup {
        group_id: listed_group_id.clone(),
        greeting: None,
        via_profile_ids: suggestion.via_friend_profile_ids.clone(),
    })?;
    wait_for_event(&bridge_b, "Bob to become a member", |event| match event {
        NetworkEvent::GroupUpdated { view }
            if view.group_id == listed_group_id
                && view.viewer_status == GroupViewerStatus::Member =>
        {
            Some(())
        }
        _ => None,
    })
    .await?;
    wait_for_event(&bridge_b, "the suggestion to clear", |event| match event {
        NetworkEvent::GroupSuggestionsUpdated { suggestions } => suggestions
            .iter()
            .all(|suggestion| suggestion.group_id != listed_group_id)
            .then_some(()),
        _ => None,
    })
    .await?;

    // Alice posts; Bob hearts it. The heart carries the group context on
    // Bob's friend-visible profile log, so Carol — a stranger to Alice and
    // the group — receives the discovery-card data pointing into the group.
    bridge_a.send(NetworkCommand::PublishGroupPost {
        group_id: listed_group_id.clone(),
        draft: GroupPostDraft {
            body: "wrens by the weir".into(),
            lifetime_secs: None,
            media: Vec::new(),
        },
    })?;
    let listed_post_id = wait_for_event(&bridge_b, "Alice's post at Bob", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.group_id == listed_group_id => view
            .posts
            .iter()
            .find(|post| post.body == "wrens by the weir")
            .map(|post| post.post_id.clone()),
        _ => None,
    })
    .await?;
    bridge_b.send(NetworkCommand::SetGroupHeart {
        group_id: listed_group_id.clone(),
        post_author_profile_id: alice_id.clone(),
        post_id: listed_post_id.clone(),
        active: true,
    })?;
    let carol_heart = wait_for_event(&bridge_c, "Bob's heart at Carol", |event| match event {
        NetworkEvent::ContactStateUpdated { profile_id, state } if *profile_id == bob_id => state
            .hearts
            .iter()
            .find(|heart| heart.post_id == listed_post_id)
            .cloned(),
        _ => None,
    })
    .await?;
    assert_eq!(
        carol_heart.group_id.as_deref(),
        Some(listed_group_id.as_str())
    );
    assert_eq!(carol_heart.group_name.as_deref(), Some("bird walks"));

    // The unlisted group: Bob joins and hearts a post there too — that
    // heart stays in-group. His own profile log never carries it.
    bridge_a.send(NetworkCommand::PublishGroupPost {
        group_id: unlisted_group_id.clone(),
        draft: GroupPostDraft {
            body: "same spot, noon".into(),
            lifetime_secs: None,
            media: Vec::new(),
        },
    })?;
    bridge_b.send(NetworkCommand::JoinGroup {
        group_id: unlisted_group_id.clone(),
        greeting: None,
        via_profile_ids: vec![alice_id.clone()],
    })?;
    let unlisted_post_id =
        wait_for_event(
            &bridge_b,
            "Bob inside the unlisted group",
            |event| match event {
                NetworkEvent::GroupUpdated { view }
                    if view.group_id == unlisted_group_id
                        && view.viewer_status == GroupViewerStatus::Member =>
                {
                    view.posts
                        .iter()
                        .find(|post| post.body == "same spot, noon")
                        .map(|post| post.post_id.clone())
                }
                _ => None,
            },
        )
        .await?;
    bridge_b.send(NetworkCommand::SetGroupHeart {
        group_id: unlisted_group_id.clone(),
        post_author_profile_id: alice_id.clone(),
        post_id: unlisted_post_id.clone(),
        active: true,
    })?;
    // The in-group heart shows in the group view...
    wait_for_event(&bridge_b, "the in-group heart", |event| match event {
        NetworkEvent::GroupUpdated { view } if view.group_id == unlisted_group_id => view
            .hearts
            .iter()
            .any(|heart| heart.post_id == unlisted_post_id)
            .then_some(()),
        _ => None,
    })
    .await?;
    // ...but Bob's own profile state carries only the listed group's heart:
    // outward heart-discovery happens iff Public AND listed (ADR-0009).
    bridge_b.send(NetworkCommand::RecoverStartup)?;
    let bob_profile_hearts =
        wait_for_event(&bridge_b, "Bob's profile state", |event| match event {
            NetworkEvent::LocalStateUpdated { state } => Some(state.hearts.clone()),
            _ => None,
        })
        .await?;
    assert!(
        bob_profile_hearts
            .iter()
            .any(|heart| heart.post_id == listed_post_id),
        "the public+listed heart is on the profile log"
    );
    assert!(
        bob_profile_hearts
            .iter()
            .all(|heart| heart.post_id != unlisted_post_id),
        "an unlisted group's heart must never reach the profile log"
    );

    Ok(())
}
