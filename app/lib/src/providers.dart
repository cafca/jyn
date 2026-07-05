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
import '../src/rust/profile.dart';
import '../src/rust/runtime.dart';
import '../src/rust/state.dart';

const int _errorLogLimit = 3;

@immutable
class AppModel {
  const AppModel({
    this.posts = const [],
    this.ghosts = const [],
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
    case JynEvent_River(:final posts, :final ghosts):
      return model.copyWith(posts: posts, ghosts: ghosts);
    case JynEvent_Profile(:final profile):
      return model.copyWith(profile: profile);
    case JynEvent_Friends(:final friends, :final pending):
      return model.copyWith(friends: friends, pendingRequests: pending);
    case JynEvent_Diagnostics(:final snapshot):
      return model.copyWith(diagnostics: snapshot);
    case JynEvent_MediaReady(:final blobHash, :final path):
      return model.copyWith(
        mediaPaths: {...model.mediaPaths, blobHash: path},
      );
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

final appModelProvider =
    NotifierProvider<AppModelNotifier, AppModel>(AppModelNotifier.new);

// Slice providers so screens only rebuild for their data.
final riverPostsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.posts)));
final ghostsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.ghosts)));
final profileProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.profile)));
final friendsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.friends)));
final pendingRequestsProvider = Provider(
    (ref) => ref.watch(appModelProvider.select((m) => m.pendingRequests)));
final diagnosticsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.diagnostics)));
final mediaPathsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.mediaPaths)));
final backgroundErrorsProvider =
    Provider((ref) => ref.watch(appModelProvider.select((m) => m.errors)));

/// One shared 1 Hz tick for countdown pills.
final clockProvider = StreamProvider<int>(
  (ref) => Stream<int>.periodic(const Duration(seconds: 1), (i) => i),
);
