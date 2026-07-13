import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/providers.dart';
import 'package:jyn/src/rust/groups.dart';
import 'package:jyn/src/rust/groups/reduce.dart';
import 'package:jyn/src/rust/groups/service.dart';
import 'package:jyn/src/rust/runtime.dart';

GroupView groupView({
  required String groupId,
  required String name,
  GroupViewerStatus viewerStatus = GroupViewerStatus.member,
  int latestActivityAt = 0,
  bool hasNewActivity = false,
  List<GroupJoinRequest> pendingRequests = const [],
}) {
  return GroupView(
    groupId: groupId,
    name: name,
    contentMode: GroupContentMode.public,
    joinMode: GroupJoinMode.open,
    discoverability: GroupDiscoverability.listed,
    createdAt: 1,
    ownerProfileId: 'owner',
    viewerStatus: viewerStatus,
    memberCount: 1,
    members: const [],
    pendingRequests: pendingRequests,
    posts: const [],
    comments: const [],
    hearts: const [],
    latestActivityAt: latestActivityAt,
    hasNewActivity: hasNewActivity,
  );
}

void main() {
  group('applyEvent', () {
    test('media paths accumulate across MediaReady events', () {
      var model = const AppModel();
      model = applyEvent(
        model,
        const JynEvent.mediaReady(blobHash: 'a', path: '/tmp/a'),
      );
      model = applyEvent(
        model,
        const JynEvent.mediaReady(blobHash: 'b', path: '/tmp/b'),
      );
      expect(model.mediaPaths, {'a': '/tmp/a', 'b': '/tmp/b'});
    });

    test('a failed fetch leaves the model unchanged', () {
      var model = const AppModel();
      model = applyEvent(
        model,
        const JynEvent.mediaFailed(blobHash: 'a', errorMessage: 'gone'),
      );
      expect(model.mediaPaths, isEmpty);
      expect(model.errors, isEmpty);
    });

    test('background errors keep only the most recent three', () {
      var model = const AppModel();
      for (var i = 0; i < 5; i++) {
        model = applyEvent(
          model,
          JynEvent.error(context: 'sync', message: 'boom $i'),
        );
      }
      expect(model.errors, ['sync: boom 2', 'sync: boom 3', 'sync: boom 4']);
    });

    test('group events fold per group, latest snapshot winning', () {
      var model = const AppModel();
      model = applyEvent(
        model,
        JynEvent.group(
          view: groupView(groupId: 'g1', name: 'reading circle'),
        ),
      );
      model = applyEvent(
        model,
        JynEvent.group(
          view: groupView(groupId: 'g2', name: 'second group'),
        ),
      );
      model = applyEvent(
        model,
        JynEvent.group(
          view: groupView(groupId: 'g1', name: 'evening reading circle'),
        ),
      );
      expect(model.groups.length, 2);
      expect(model.groups['g1']!.name, 'evening reading circle');
      expect(model.groups['g2']!.name, 'second group');
    });

    test('river snapshots carry the digest doors', () {
      var model = const AppModel();
      model = applyEvent(
        model,
        const JynEvent.river(
          posts: [],
          ghosts: [],
          doors: [
            GroupDigestDoor(
              groupId: 'g1',
              name: 'reading circle',
              latestActivityAt: 10,
            ),
          ],
          groupCards: [],
        ),
      );
      expect(model.doors.single.groupId, 'g1');
      // A later river snapshot replaces the doors.
      model = applyEvent(
        model,
        const JynEvent.river(posts: [], ghosts: [], doors: [], groupCards: []),
      );
      expect(model.doors, isEmpty);
    });

    test('friends snapshots replace rather than merge', () {
      var model = const AppModel();
      model = applyEvent(
        model,
        const JynEvent.friends(
          friends: [
            FriendEntry(
              profileId: 'anna',
              displayName: 'Anna',
              followsMeBack: true,
            ),
          ],
          pending: [],
        ),
      );
      model = applyEvent(
        model,
        const JynEvent.friends(friends: [], pending: []),
      );
      expect(model.friends, isEmpty);
    });
  });

  group('group slice providers', () {
    test('myGroups lists member groups only, most recently active first', () {
      final container = ProviderContainer(
        overrides: [jynEventsProvider.overrideWithValue(const Stream.empty())],
      );
      addTearDown(container.dispose);
      final notifier = container.read(appModelProvider.notifier);
      for (final view in [
        groupView(
          groupId: 'mine-quiet',
          name: 'quiet',
          viewerStatus: GroupViewerStatus.owner,
          latestActivityAt: 5,
        ),
        groupView(groupId: 'mine-busy', name: 'busy', latestActivityAt: 50),
        groupView(
          groupId: 'visited',
          name: 'visited',
          viewerStatus: GroupViewerStatus.nonMember,
          latestActivityAt: 99,
        ),
        groupView(
          groupId: 'asked',
          name: 'asked',
          viewerStatus: GroupViewerStatus.pending,
          latestActivityAt: 98,
        ),
      ]) {
        notifier.onEvent(JynEvent.group(view: view));
      }

      final mine = container.read(myGroupsProvider);
      expect(mine.map((g) => g.groupId).toList(), ['mine-busy', 'mine-quiet']);
      // A merely visited or requested group is not "my group".
      expect(container.read(groupProvider('visited'))!.name, 'visited');
    });
  });
}
