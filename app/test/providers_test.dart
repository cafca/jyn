import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/providers.dart';
import 'package:jyn/src/rust/runtime.dart';

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
}
