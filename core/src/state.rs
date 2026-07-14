//! UI-side feed state: the river, materialized from the local profile's
//! reduced state, private posts, and every synced friend's reduced state.

use std::collections::HashMap;

use crate::domain::{ReducedPost, ReducedProfileState};
use crate::local_stores::KeepRecord;

/// A named heart on a river post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiverHeart {
    pub hearter_profile_id: String,
    pub hearter_display_name: String,
}

/// A comment under a river post, joined from the commenter's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiverComment {
    pub commenter_profile_id: String,
    pub commenter_display_name: String,
    pub body: String,
    pub created_at: u64,
}

/// A greyed-out discovery teaser: a friend's heart points at a post by
/// someone who isn't a friend yet. The content itself is not shown — we
/// don't sync the author — only the door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostCard {
    pub carrier_display_name: String,
    pub author_profile_id: String,
}

/// A named discovery card from a friend's heart on a **public + listed**
/// group post: "♥ Bob, in *Group X*" — a pointer into the group place, the
/// post is not copied or moved (ADR-0009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDiscoveryCard {
    pub carrier_profile_id: String,
    pub carrier_display_name: String,
    pub group_id: String,
    pub group_name: String,
}

/// One post as the river renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiverPost {
    pub author_profile_id: String,
    pub author_display_name: String,
    pub is_self: bool,
    pub post: ReducedPost,
    /// Named hearts from everyone whose stream we see (never a bare count).
    pub hearts: Vec<RiverHeart>,
    pub hearted_by_me: bool,
    pub comments: Vec<RiverComment>,
    pub kept_by_me: bool,
}

/// The interleaved feed and the state it is derived from.
///
/// Sources are applied from network events; `materialize` rebuilds the
/// reverse-chronological river with expired posts filtered out. `next_expiry`
/// lets the tick system know when the river changes shape without any event
/// arriving (a lifetime running out).
#[derive(Default)]
pub struct RiverState {
    own: Option<ReducedProfileState>,
    own_display_name: Option<String>,
    private_posts: Vec<ReducedPost>,
    by_friend: HashMap<String, ReducedProfileState>,
    keeps: Vec<KeepRecord>,
    pub river: Vec<RiverPost>,
    pub ghosts: Vec<GhostCard>,
    /// Friends' hearts on public+listed group posts, one card per group.
    /// Membership filtering happens in the runtime (which knows the
    /// viewer's groups).
    pub group_cards: Vec<GroupDiscoveryCard>,
    next_expiry: Option<u64>,
    dirty: bool,
}

impl RiverState {
    pub fn apply_local_state(&mut self, state: ReducedProfileState) {
        self.own = Some(state);
        self.dirty = true;
    }

    /// The profile's display name as known before any operation exists
    /// (from `ProfileLoaded`), used for own posts in the river.
    pub fn set_own_display_name(&mut self, display_name: impl Into<String>) {
        self.own_display_name = Some(display_name.into());
        self.dirty = true;
    }

    pub fn apply_private_posts(&mut self, posts: Vec<ReducedPost>) {
        self.private_posts = posts;
        self.dirty = true;
    }

    pub fn apply_keeps(&mut self, keeps: Vec<KeepRecord>) {
        self.keeps = keeps;
        self.dirty = true;
    }

    pub fn apply_contact_state(&mut self, profile_id: String, state: ReducedProfileState) {
        self.by_friend.insert(profile_id, state);
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn own_state(&self) -> Option<&ReducedProfileState> {
        self.own.as_ref()
    }

    /// Do we follow this profile (accepted them / completed the friendship)?
    pub fn follows(&self, profile_id: &str) -> bool {
        self.own
            .as_ref()
            .map(|own| {
                own.followed_profile_ids
                    .iter()
                    .any(|followed| followed == profile_id)
            })
            .unwrap_or(false)
    }

    /// The synced reduced state of a contact, if any (also present for
    /// requested-but-not-yet-accepted profiles).
    pub fn contact_state(&self, profile_id: &str) -> Option<&ReducedProfileState> {
        self.by_friend.get(profile_id)
    }

    pub fn contact_display_name(&self, profile_id: &str) -> String {
        self.by_friend
            .get(profile_id)
            .and_then(|state| state.display_name.clone())
            .unwrap_or_else(|| short_id(profile_id))
    }

    /// True when a lifetime has run out since the last materialization, so
    /// the river needs to be rebuilt (and stores drained) even without a
    /// network event.
    pub fn expiry_due(&self, now: u64) -> bool {
        self.next_expiry.is_some_and(|at| at <= now)
    }

    /// Rebuilds the river: own + private + friends' posts, expired filtered,
    /// newest first, with hearts and comments joined on from every stream
    /// we can see.
    pub fn materialize(&mut self, now: u64) {
        let mut river = Vec::new();
        let mut next_expiry: Option<u64> = None;
        let track = |post: &ReducedPost, next_expiry: &mut Option<u64>| {
            if let Some(expires_at) = post.expires_at {
                if expires_at > now {
                    *next_expiry = Some(next_expiry.map_or(expires_at, |at| at.min(expires_at)));
                }
            }
        };

        let followed = self
            .own
            .as_ref()
            .map(|own| own.followed_profile_ids.clone())
            .unwrap_or_default();
        // Circle members (friends-of-friends): their circles posts decrypt
        // for us when they added us to their circle, and those posts belong
        // in the river even though we don't follow them. Friends-only posts
        // of theirs never decrypt, so nothing over-broad can leak through.
        let my_id = self
            .own
            .as_ref()
            .map(|own| own.profile_id.clone())
            .unwrap_or_default();
        let circle_authors: std::collections::HashSet<String> = self
            .by_friend
            .values()
            .filter(|state| followed.contains(&state.profile_id))
            .flat_map(|state| state.followed_profile_ids.iter().cloned())
            .filter(|profile_id| !followed.contains(profile_id) && *profile_id != my_id)
            .collect();
        let visible_author = |profile_id: &str| {
            followed.iter().any(|followed| followed == profile_id)
                || circle_authors.contains(profile_id)
        };
        let own_name = self
            .own
            .as_ref()
            .and_then(|own| own.display_name.clone())
            .or_else(|| self.own_display_name.clone())
            .unwrap_or_else(|| "you".to_owned());
        let own_id = self
            .own
            .as_ref()
            .map(|own| own.profile_id.clone())
            .unwrap_or_default();

        // Hearts and comments live in the interactor's stream; index them by
        // the post they target. Only streams we legitimately see contribute:
        // our own and those of friends we follow.
        let mut hearts_by_post: HashMap<(String, String), Vec<RiverHeart>> = HashMap::new();
        let mut comments_by_post: HashMap<(String, String), Vec<RiverComment>> = HashMap::new();
        let mut my_hearts: Vec<(String, String)> = Vec::new();
        {
            let mut index_interactions =
                |state: &ReducedProfileState, interactor_name: &str, is_me: bool| {
                    for heart in &state.hearts {
                        let key = (heart.post_author_profile_id.clone(), heart.post_id.clone());
                        if is_me {
                            my_hearts.push(key.clone());
                        }
                        hearts_by_post.entry(key).or_default().push(RiverHeart {
                            hearter_profile_id: state.profile_id.clone(),
                            hearter_display_name: interactor_name.to_owned(),
                        });
                    }
                    for comment in &state.comments {
                        let key = (
                            comment.post_author_profile_id.clone(),
                            comment.post_id.clone(),
                        );
                        comments_by_post.entry(key).or_default().push(RiverComment {
                            commenter_profile_id: state.profile_id.clone(),
                            commenter_display_name: interactor_name.to_owned(),
                            body: comment.body.clone(),
                            created_at: comment.created_at,
                        });
                    }
                };

            if let Some(own) = &self.own {
                index_interactions(own, &own_name, true);
            }
            for state in self.by_friend.values() {
                if !visible_author(&state.profile_id) {
                    continue;
                }
                let name = state
                    .display_name
                    .clone()
                    .unwrap_or_else(|| short_id(&state.profile_id));
                index_interactions(state, &name, false);
            }
        }
        for comments in comments_by_post.values_mut() {
            comments.sort_by_key(|comment| comment.created_at);
        }

        let kept: Vec<(String, String)> = self
            .keeps
            .iter()
            .map(|keep| (keep.author_profile_id.clone(), keep.post_id.clone()))
            .collect();

        let push_post = |post: &ReducedPost,
                         author_id: &str,
                         author_name: &str,
                         is_self: bool,
                         river: &mut Vec<RiverPost>,
                         next_expiry: &mut Option<u64>| {
            track(post, next_expiry);
            let key = (author_id.to_owned(), post.post_id.clone());
            river.push(RiverPost {
                author_profile_id: author_id.to_owned(),
                author_display_name: author_name.to_owned(),
                is_self,
                hearts: hearts_by_post.get(&key).cloned().unwrap_or_default(),
                hearted_by_me: my_hearts.contains(&key),
                comments: comments_by_post.get(&key).cloned().unwrap_or_default(),
                kept_by_me: kept.contains(&key),
                post: post.clone(),
            });
        };

        if let Some(own) = &self.own {
            for post in own.active_posts(now) {
                push_post(post, &own_id, &own_name, true, &mut river, &mut next_expiry);
            }
        }

        for post in &self.private_posts {
            if post.is_expired(now) {
                continue;
            }
            push_post(
                post,
                &post.profile_id.clone(),
                &own_name,
                true,
                &mut river,
                &mut next_expiry,
            );
        }

        // Friends we follow and circle members flow into the river; a
        // synced-but-unaccepted topic (someone we merely requested) or an
        // unfriended, out-of-circle profile's lingering state contributes
        // nothing.
        for state in self.by_friend.values() {
            if !visible_author(&state.profile_id) {
                continue;
            }
            let name = state
                .display_name
                .clone()
                .unwrap_or_else(|| short_id(&state.profile_id));
            for post in state.active_posts(now) {
                push_post(
                    post,
                    &state.profile_id,
                    &name,
                    false,
                    &mut river,
                    &mut next_expiry,
                );
            }
        }

        river.sort_by(|left, right| {
            right
                .post
                .created_at
                .cmp(&left.post.created_at)
                .then_with(|| left.post.post_id.cmp(&right.post.post_id))
        });

        // Discovery ghosts: hearts cast by friends on posts whose authors we
        // don't follow. One door per unknown author, carried by whoever
        // hearted them first.
        let visible_authors: std::collections::HashSet<&str> = river
            .iter()
            .map(|post| post.author_profile_id.as_str())
            .collect();
        let mut ghosts: Vec<GhostCard> = Vec::new();
        let mut group_cards: Vec<GroupDiscoveryCard> = Vec::new();
        for state in self.by_friend.values() {
            if !followed.contains(&state.profile_id) {
                continue;
            }
            let carrier = state
                .display_name
                .clone()
                .unwrap_or_else(|| short_id(&state.profile_id));
            for heart in &state.hearts {
                // A heart carrying group context surfaces as a named door
                // into the group (ADR-0009), never as a ghost — the post
                // lives in the group, not on the author's profile.
                if let (Some(group_id), Some(group_name)) = (&heart.group_id, &heart.group_name) {
                    if !group_cards.iter().any(|card| &card.group_id == group_id) {
                        group_cards.push(GroupDiscoveryCard {
                            carrier_profile_id: state.profile_id.clone(),
                            carrier_display_name: carrier.clone(),
                            group_id: group_id.clone(),
                            group_name: group_name.clone(),
                        });
                    }
                    continue;
                }
                let author = heart.post_author_profile_id.as_str();
                if author == own_id
                    || followed.iter().any(|followed| followed == author)
                    || visible_authors.contains(author)
                    || ghosts.iter().any(|ghost| ghost.author_profile_id == author)
                {
                    continue;
                }
                ghosts.push(GhostCard {
                    carrier_display_name: carrier.clone(),
                    author_profile_id: author.to_owned(),
                });
            }
        }
        ghosts.sort_by(|left, right| left.author_profile_id.cmp(&right.author_profile_id));
        group_cards.sort_by(|left, right| left.group_id.cmp(&right.group_id));

        self.river = river;
        self.ghosts = ghosts;
        self.group_cards = group_cards;
        self.next_expiry = next_expiry;
        self.dirty = false;
    }
}

fn short_id(profile_id: &str) -> String {
    profile_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Visibility;

    fn post(post_id: &str, created_at: u64, expires_at: Option<u64>) -> ReducedPost {
        ReducedPost {
            profile_id: "author".into(),
            post_id: post_id.into(),
            body: "body".into(),
            media: Vec::new(),
            visibility: Visibility::Friends,
            expires_at,
            created_at,
            edited: false,
        }
    }

    fn state_with_posts(profile_id: &str, posts: Vec<ReducedPost>) -> ReducedProfileState {
        ReducedProfileState {
            profile_id: profile_id.into(),
            display_name: Some(format!("{profile_id}-name")),
            bio: String::new(),
            default_visibility: Visibility::Friends,
            default_lifetime_secs: None,
            posts,
            followed_profile_ids: Vec::new(),
            hearts: Vec::new(),
            comments: Vec::new(),
            pending_requests: Vec::new(),
            tombstoned_post_ids: Vec::new(),
            advertised_groups: Vec::new(),
        }
    }

    #[test]
    fn a_friends_group_heart_becomes_a_discovery_card_not_a_ghost() {
        use crate::domain::HeartRef;

        let mut river = RiverState::default();
        river.set_own_display_name("Me");
        let mut own = state_with_posts("me", Vec::new());
        own.followed_profile_ids = vec!["anna".into()];
        river.apply_local_state(own);

        // Anna hearts a post in a public+listed group (group context set)
        // and a stranger's plain post (no group context).
        let mut anna = state_with_posts("anna", Vec::new());
        anna.hearts = vec![
            HeartRef {
                post_author_profile_id: "member-x".into(),
                post_id: "group-post".into(),
                recorded_at: 10,
                group_id: Some("g-1".into()),
                group_name: Some("reading circle".into()),
            },
            HeartRef {
                post_author_profile_id: "stranger".into(),
                post_id: "plain-post".into(),
                recorded_at: 11,
                group_id: None,
                group_name: None,
            },
        ];
        river.apply_contact_state("anna".into(), anna);

        river.materialize(45);
        // The group heart surfaces as a named card into the group, framed
        // with provenance — and never doubles as a ghost door.
        assert_eq!(
            river.group_cards,
            vec![GroupDiscoveryCard {
                carrier_profile_id: "anna".into(),
                carrier_display_name: "anna-name".into(),
                group_id: "g-1".into(),
                group_name: "reading circle".into(),
            }]
        );
        assert_eq!(river.ghosts.len(), 1);
        assert_eq!(river.ghosts[0].author_profile_id, "stranger");
    }

    #[test]
    fn river_merges_sources_reverse_chronologically_and_drains_expired() {
        let mut river = RiverState::default();
        river.set_own_display_name("Me");
        let mut own = state_with_posts(
            "me",
            vec![post("own-1", 10, None), post("own-2", 40, Some(100))],
        );
        own.followed_profile_ids = vec!["anna".into()];
        river.apply_local_state(own);
        river.apply_private_posts(vec![post("private-1", 30, None)]);
        river.apply_contact_state(
            "anna".into(),
            state_with_posts("anna", vec![post("anna-1", 20, Some(50))]),
        );
        // A synced topic we do NOT follow (a pending request target, or an
        // unfriended profile) never reaches the river.
        river.apply_contact_state(
            "stranger".into(),
            state_with_posts("stranger", vec![post("stranger-1", 35, None)]),
        );

        assert!(river.is_dirty());
        river.materialize(45);
        let ids: Vec<_> = river
            .river
            .iter()
            .map(|p| p.post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["own-2", "private-1", "anna-1", "own-1"]);
        assert!(!river.is_dirty());

        // anna-1 expires at 50: due at 50, gone after re-materializing.
        assert!(!river.expiry_due(49));
        assert!(river.expiry_due(50));
        river.materialize(50);
        let ids: Vec<_> = river
            .river
            .iter()
            .map(|p| p.post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["own-2", "private-1", "own-1"]);

        // own-2 is the next lifetime to run out.
        assert!(river.expiry_due(100));
        river.materialize(100);
        let ids: Vec<_> = river
            .river
            .iter()
            .map(|p| p.post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["private-1", "own-1"]);
        // Nothing left to expire.
        assert!(!river.expiry_due(u64::MAX));
    }

    #[test]
    fn circle_members_posts_flow_into_the_river() {
        let mut river = RiverState::default();
        river.set_own_display_name("Me");
        let mut own = state_with_posts("me", Vec::new());
        own.followed_profile_ids = vec!["anna".into()];
        river.apply_local_state(own);

        // Anna (a friend) follows Carol — Carol is in our circle. Whatever of
        // Carol's stream we could decrypt (her circles posts) is visible.
        let mut anna = state_with_posts("anna", Vec::new());
        anna.followed_profile_ids = vec!["me".into(), "carol".into()];
        river.apply_contact_state("anna".into(), anna);
        river.apply_contact_state(
            "carol".into(),
            state_with_posts("carol", vec![post("carol-1", 20, None)]),
        );
        // A synced profile nobody's follow list names stays out.
        river.apply_contact_state(
            "stranger".into(),
            state_with_posts("stranger", vec![post("stranger-1", 30, None)]),
        );

        river.materialize(45);
        let ids: Vec<_> = river
            .river
            .iter()
            .map(|p| p.post.post_id.as_str())
            .collect();
        assert_eq!(ids, vec!["carol-1"]);
    }
}
