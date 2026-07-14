import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/rust/domain.dart';
import 'package:jyn/src/widgets/media_playback.dart';
import 'package:jyn/src/widgets/video_attachment.dart';

import 'fake_playback.dart';

// The rendered video surface needs native libmpv, so these tests cover the
// pre-render branches (downloading / loading) and the wiring that opens the
// blob. Real playback is verified manually on a macOS release build.
void main() {
  const attachment = MediaAttachment(
    blobHash: 'abc',
    kind: MediaKind.video,
    mime: 'video/mp4',
    byteLen: 1234,
  );

  Future<void> pumpVideo(
    WidgetTester tester, {
    required String? path,
    required MediaPlaybackFactory factory,
  }) {
    return tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: SizedBox(
              width: 200,
              height: 250,
              child: VideoAttachment(
                attachment: attachment,
                path: path,
                playerKey: 'post:abc',
                playbackFactory: factory,
              ),
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('shows the downloading placeholder while the blob is remote', (
    tester,
  ) async {
    final fake = FakePlayback();
    await pumpVideo(tester, path: null, factory: () => fake);

    expect(find.byIcon(Icons.downloading), findsOneWidget);
    expect(fake.calls, isEmpty);
  });

  testWidgets('opens the file and shows a loading spinner once it is local', (
    tester,
  ) async {
    final fake = FakePlayback();
    await pumpVideo(tester, path: '/tmp/clip.mp4', factory: () => fake);
    // Not pumpAndSettle: the loading spinner animates forever. A couple of
    // frames is enough to flush _init's async open.
    await tester.pump();
    await tester.pump();

    expect(fake.openedPath, '/tmp/clip.mp4');
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(find.byIcon(Icons.downloading), findsNothing);
  });
}
