/// The Rust core's event stream, folded into one immutable [AppModel] that
/// every screen derives from. Commands go the other way as awaitable calls
/// directly on the generated API (see `runGuarded` for the error pattern).
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../src/rust/api/lifecycle.dart' as rust;
import '../src/rust/diagnostics.dart';
import '../src/rust/domain.dart';
import '../src/rust/groups/service.dart';
import '../src/rust/profile.dart';
import '../src/rust/runtime.dart';
import '../src/rust/state.dart';

const int _errorLogLimit = 3;

@immutable
class AppModel {
  const AppModel({
    this.posts = const [],
    this.ghosts = const [],
    this.doors = const [],
    this.groupCards = const [],
    this.groups = const {},
    this.groupSuggestions = const [],
    this.profile,
    this.friends = const [],
    this.pendingRequests = const [],
    this.diagnostics,
    this.mediaPaths = const {},
    this.errors = const [],
  });

  /// The materialized river, newest first, expired posts already drained.
  final List<RiverPost> posts;

  /// Discovery doors: friends' hearts on posts by authors we don't follow.
  final List<GhostCard> ghosts;

  /// One digest door per member-group with new activity, river-sorted.
  final List<GroupDigestDoor> doors;

  /// Friends' hearts on public+listed group posts: named doors into groups.
  final List<GroupDiscoveryCard> groupCards;

  /// GroupId → the group's latest viewer-filtered state.
  final Map<String, GroupView> groups;

  /// Groups friends advertise that the local user hasn't joined.
  final List<GroupSuggestion> groupSuggestions;

  /// The local profile; null until the first Profile event.
  final UserProfile? profile;

  final List<FriendEntry> friends;
  final List<PendingFriendRequest> pendingRequests;
  final DiagnosticsSnapshot? diagnostics;

  /// Blob hash → local file path, filled by MediaReady events.
  final Map<String, String> mediaPaths;

  /// Recent background errors (user-action failures throw at call sites).
  final List<String> errors;

  AppModel copyWith({
    List<RiverPost>? posts,
    List<GhostCard>? ghosts,
    List<GroupDigestDoor>? doors,
    List<GroupDiscoveryCard>? groupCards,
    Map<String, GroupView>? groups,
    List<GroupSuggestion>? groupSuggestions,
    UserProfile? profile,
    List<FriendEntry>? friends,
    List<PendingFriendRequest>? pendingRequests,
    DiagnosticsSnapshot? diagnostics,
    Map<String, String>? mediaPaths,
    List<String>? errors,
  }) {
    return AppModel(
      posts: posts ?? this.posts,
      ghosts: ghosts ?? this.ghosts,
      doors: doors ?? this.doors,
      groupCards: groupCards ?? this.groupCards,
      groups: groups ?? this.groups,
      groupSuggestions: groupSuggestions ?? this.groupSuggestions,
      profile: profile ?? this.profile,
      friends: friends ?? this.friends,
      pendingRequests: pendingRequests ?? this.pendingRequests,
      diagnostics: diagnostics ?? this.diagnostics,
      mediaPaths: mediaPaths ?? this.mediaPaths,
      errors: errors ?? this.errors,
    );
  }
}

class AppModelNotifier extends Notifier<AppModel> {
  StreamSubscription<JynEvent>? _subscription;

  @override
  AppModel build() {
    _subscription?.cancel();
    _subscription = ref.watch(jynEventsProvider).listen(onEvent);
    ref.onDispose(() => _subscription?.cancel());
    return const AppModel();
  }

  @visibleForTesting
  void onEvent(JynEvent event) {
    state = applyEvent(state, event);
  }
}

/// Pure fold of one runtime event into the model — the Dart-side logic the
/// spec wants tested.
AppModel applyEvent(AppModel model, JynEvent event) {
  switch (event) {
    case JynEvent_River(
      :final posts,
      :final ghosts,
      :final doors,
      :final groupCards,
    ):
      return model.copyWith(
        posts: posts,
        ghosts: ghosts,
        doors: doors,
        groupCards: groupCards,
      );
    case JynEvent_Group(:final view):
      return model.copyWith(groups: {...model.groups, view.groupId: view});
    case JynEvent_GroupSuggestions(:final suggestions):
      return model.copyWith(groupSuggestions: suggestions);
    case JynEvent_Profile(:final profile):
      return model.copyWith(profile: profile);
    case JynEvent_Friends(:final friends, :final pending):
      return model.copyWith(friends: friends, pendingRequests: pending);
    case JynEvent_Diagnostics(:final snapshot):
      return model.copyWith(diagnostics: snapshot);
    case JynEvent_MediaReady(:final blobHash, :final path):
      return model.copyWith(mediaPaths: {...model.mediaPaths, blobHash: path});
    case JynEvent_MediaFailed():
      // The attachment widget retries on tap; nothing to record.
      return model;
    case JynEvent_Error(:final context, :final message):
      final errors = [...model.errors, '$context: $message'];
      return model.copyWith(
        errors: errors.length > _errorLogLimit
            ? errors.sublist(errors.length - _errorLogLimit)
            : errors,
      );
  }
}

/// The single subscription to the Rust event stream. Overridden in tests.
final jynEventsProvider = Provider<Stream<JynEvent>>((ref) => rust.events());

final appModelProvider = NotifierProvider<AppModelNotifier, AppModel>(
  AppModelNotifier.new,
);

// Slice providers so screens only rebuild for their data.
final riverPostsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.posts)),
);
final ghostsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.ghosts)),
);
final groupDoorsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.doors)),
);
final groupCardsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.groupCards)),
);
final groupSuggestionsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.groupSuggestions)),
);
final groupsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.groups)),
);

/// One group's latest state, for the place and admin screens.
final groupProvider = Provider.family<GroupView?, String>(
  (ref, groupId) => ref.watch(groupsProvider.select((groups) => groups[groupId])),
);

/// The Groups hub list: groups the local user belongs to, most recently
/// active first.
final myGroupsProvider = Provider<List<GroupView>>((ref) {
  final groups = ref.watch(groupsProvider).values.where(
    (view) =>
        view.viewerStatus == GroupViewerStatus.owner ||
        view.viewerStatus == GroupViewerStatus.member,
  ).toList()
    ..sort(
      (a, b) => b.latestActivityAt != a.latestActivityAt
          ? b.latestActivityAt.compareTo(a.latestActivityAt)
          : a.name.compareTo(b.name),
    );
  return groups;
});
final profileProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.profile)),
);
final friendsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.friends)),
);
final pendingRequestsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.pendingRequests)),
);
final diagnosticsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.diagnostics)),
);
final mediaPathsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.mediaPaths)),
);
final backgroundErrorsProvider = Provider(
  (ref) => ref.watch(appModelProvider.select((m) => m.errors)),
);

/// One shared 1 Hz tick for countdown pills.
final clockProvider = StreamProvider<int>(
  (ref) => Stream<int>.periodic(const Duration(seconds: 1), (i) => i),
);

/// The post currently loaded into the composer for editing (a post-card
/// "edit" action sets it; the composer clears it on save/cancel).
class EditingPostNotifier extends Notifier<ReducedPost?> {
  @override
  ReducedPost? build() => null;

  void start(ReducedPost post) => state = post;

  void clear() => state = null;
}

final editingPostProvider = NotifierProvider<EditingPostNotifier, ReducedPost?>(
  EditingPostNotifier.new,
);
