import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/widgets/media_playback.dart';
import 'package:jyn/src/widgets/voice_note_player.dart';

import 'fake_playback.dart';

void main() {
  Future<void> pumpPlayer(
    WidgetTester tester, {
    required String? path,
    required MediaPlaybackFactory factory,
  }) {
    return tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: SizedBox(
              width: 300,
              child: VoiceNotePlayer(
                waveform: const [10, 200, 120, 40, 255],
                durationMs: 1000,
                path: path,
                playbackFactory: factory,
              ),
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('shows the downloading state and opens nothing while fetching', (
    tester,
  ) async {
    final fake = FakePlayback();
    await pumpPlayer(tester, path: null, factory: () => fake);

    expect(find.byIcon(Icons.downloading), findsOneWidget);
    // The play circle is inert while fetching, so a tap opens no player.
    await tester.tap(find.byType(GestureDetector).first);
    await tester.pumpAndSettle();
    expect(fake.calls, isEmpty);
  });

  testWidgets('tapping play opens the file and plays, then pauses', (
    tester,
  ) async {
    final fake = FakePlayback();
    await pumpPlayer(tester, path: '/tmp/note.wav', factory: () => fake);

    expect(find.byIcon(Icons.play_arrow), findsOneWidget);

    await tester.tap(find.byType(GestureDetector).first);
    await tester.pumpAndSettle();
    expect(fake.openedPath, '/tmp/note.wav');
    expect(fake.calls, containsAllInOrder(['open', 'play']));

    // The engine reports it is playing; the button should invite a pause.
    fake.emitPlaying(true);
    await tester.pumpAndSettle();
    expect(find.byIcon(Icons.pause), findsOneWidget);

    await tester.tap(find.byType(GestureDetector).first);
    await tester.pumpAndSettle();
    expect(fake.calls.last, 'pause');
  });

  testWidgets('a completed note replays from the top', (tester) async {
    final fake = FakePlayback();
    await pumpPlayer(tester, path: '/tmp/note.wav', factory: () => fake);

    await tester.tap(find.byType(GestureDetector).first);
    await tester.pumpAndSettle();
    fake.emitPlaying(true);
    fake.emitCompleted(true);
    await tester.pumpAndSettle();

    // A finished note shows as paused (invites a replay).
    expect(find.byIcon(Icons.play_arrow), findsOneWidget);

    fake.seekedTo = null;
    await tester.tap(find.byType(GestureDetector).first);
    await tester.pumpAndSettle();
    expect(fake.seekedTo, Duration.zero);
    expect(fake.calls.last, 'play');
  });

  testWidgets('tapping the waveform centre seeks to the middle', (
    tester,
  ) async {
    final fake = FakePlayback();
    await pumpPlayer(tester, path: '/tmp/note.wav', factory: () => fake);

    // The waveform is the second (last) gesture area; a centre tap is 50%.
    // With no decoded duration yet, total falls back to the 1000ms summary.
    await tester.tap(find.byType(GestureDetector).last);
    await tester.pumpAndSettle();
    expect(fake.calls, contains('seek'));
    expect(fake.seekedTo, const Duration(milliseconds: 500));
  });
}
