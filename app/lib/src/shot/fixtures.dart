/// Deterministic fixture data for the screenshot harness: every post
/// archetype and lifetime state the design cares about, without booting
/// a second peer.
library;

import 'dart:typed_data';

import '../rust/diagnostics.dart';
import '../rust/domain.dart';
import '../rust/profile.dart';
import '../rust/runtime.dart';
import '../rust/state.dart';

const shotSelfId = 'a1b2c3d4self';

UserProfile shotProfile({required int now, bool onboarded = true}) {
  return UserProfile(
    version: 1,
    profileId: shotSelfId,
    displayName: 'Ren Okafor',
    bio: 'gardening, tape loops, small rivers.',
    defaultVisibility: Visibility.circles,
    defaultLifetimeSecs: 24 * 3600,
    onboarded: onboarded,
    createdAt: now - 90 * 24 * 3600,
    updatedAt: now - 3600,
  );
}

/// The full event replay the harness feeds through the normal provider
/// fold — the screens render exactly as they would live.
List<JynEvent> shotEvents({
  required int now,
  required String photoPath,
  required String audioPath,
}) {
  final photo = MediaAttachment(
    kind: MediaKind.photo,
    blobHash: 'shot-photo-blob',
    byteLen: 480000,
    mime: 'image/png',
    width: 800,
    height: 1000,
  );
  final audio = MediaAttachment(
    kind: MediaKind.audio,
    blobHash: 'shot-audio-blob',
    byteLen: 96000,
    mime: 'audio/wav',
    durationMs: 12000,
    waveform: Uint8List.fromList(
      List.generate(40, (i) => 60 + ((i * 47) % 180)),
    ),
  );

  final posts = <RiverPost>[
    // Photo post, ebbing: the ring on media, hearts overflow, comments.
    RiverPost(
      authorProfileId: 'mira0000',
      authorDisplayName: 'Mira Kest',
      isSelf: false,
      post: ReducedPost(
        profileId: 'mira0000',
        postId: 'post-photo',
        body: 'low tide at the spillway',
        media: [photo],
        visibility: Visibility.circles,
        expiresAt: now + 34 * 3600,
        createdAt: now + 34 * 3600 - 7 * 24 * 3600,
        edited: false,
      ),
      hearts: const [
        RiverHeart(hearterProfileId: 'soren000', hearterDisplayName: 'Soren'),
        RiverHeart(hearterProfileId: 'bo000000', hearterDisplayName: 'Bo'),
        RiverHeart(hearterProfileId: 'june0000', hearterDisplayName: 'June'),
      ],
      heartedByMe: true,
      comments: [
        RiverComment(
          commenterProfileId: 'soren000',
          commenterDisplayName: 'Soren',
          body: 'that light!',
          createdAt: now - 3600,
        ),
        RiverComment(
          commenterProfileId: shotSelfId,
          commenterDisplayName: 'Ren Okafor',
          body: 'saving this spot',
          createdAt: now - 1800,
        ),
      ],
      keptByMe: false,
    ),
    // Audio post, settled: chip in the author row, kept by me.
    RiverPost(
      authorProfileId: 'soren000',
      authorDisplayName: 'Soren Lieb',
      isSelf: false,
      post: ReducedPost(
        profileId: 'soren000',
        postId: 'post-audio',
        body: '',
        media: [audio],
        visibility: Visibility.friends,
        expiresAt: null,
        createdAt: now - 26 * 3600,
        edited: false,
      ),
      hearts: const [
        RiverHeart(hearterProfileId: 'mira0000', hearterDisplayName: 'Mira'),
      ],
      heartedByMe: true,
      comments: const [],
      keptByMe: true,
    ),
    // Own text post, draining: the amber dashed card.
    RiverPost(
      authorProfileId: shotSelfId,
      authorDisplayName: 'Ren Okafor',
      isSelf: true,
      post: ReducedPost(
        profileId: shotSelfId,
        postId: 'post-draining',
        body: 'thought from the towpath — gone by morning unless kept.',
        media: const [],
        visibility: Visibility.circles,
        expiresAt: now + 4 * 3600,
        createdAt: now + 4 * 3600 - 24 * 3600,
        edited: true,
      ),
      hearts: const [
        RiverHeart(hearterProfileId: 'june0000', hearterDisplayName: 'June'),
      ],
      heartedByMe: false,
      comments: const [],
      keptByMe: false,
    ),
    // Own settled text post: permanent card.
    RiverPost(
      authorProfileId: shotSelfId,
      authorDisplayName: 'Ren Okafor',
      isSelf: true,
      post: ReducedPost(
        profileId: shotSelfId,
        postId: 'post-settled',
        body:
            'The river does not hurry, and still everything that must '
            'arrive, arrives.',
        media: const [],
        visibility: Visibility.public,
        expiresAt: null,
        createdAt: now - 12 * 24 * 3600,
        edited: false,
      ),
      hearts: const [
        RiverHeart(hearterProfileId: 'mira0000', hearterDisplayName: 'Mira'),
        RiverHeart(hearterProfileId: 'wen00000', hearterDisplayName: 'Wen'),
      ],
      heartedByMe: false,
      comments: const [],
      keptByMe: false,
    ),
    // Plain ebbing text post from a friend.
    RiverPost(
      authorProfileId: 'june0000',
      authorDisplayName: 'June Park',
      isSelf: false,
      post: ReducedPost(
        profileId: 'june0000',
        postId: 'post-text',
        body: 'kettle on, rain out, tape loop going',
        media: const [],
        visibility: Visibility.friends,
        expiresAt: now + 20 * 3600,
        createdAt: now + 20 * 3600 - 24 * 3600,
        edited: false,
      ),
      hearts: const [],
      heartedByMe: false,
      comments: const [],
      keptByMe: false,
    ),
  ];

  const ghosts = [
    GhostCard(carrierDisplayName: 'June', authorProfileId: 'stranger01'),
  ];

  const friends = [
    FriendEntry(
      profileId: 'mira0000',
      displayName: 'Mira Kest',
      followsMeBack: true,
    ),
    FriendEntry(
      profileId: 'soren000',
      displayName: 'Soren Lieb',
      followsMeBack: true,
    ),
    FriendEntry(profileId: 'bo000000', displayName: 'Bo', followsMeBack: true),
    FriendEntry(
      profileId: 'june0000',
      displayName: 'June Park',
      followsMeBack: true,
    ),
    FriendEntry(
      profileId: 'wen00000',
      displayName: 'Wen Liu',
      followsMeBack: true,
    ),
    FriendEntry(
      profileId: 'tal00000',
      displayName: 'Tal Noor',
      followsMeBack: true,
    ),
    FriendEntry(
      profileId: 'pia00000',
      displayName: 'Pia Voss',
      followsMeBack: false,
    ),
  ];

  final pending = [
    PendingFriendRequest(
      requesterProfileId: 'ada00000',
      requesterDisplayName: 'Ada',
      recordedAt: now - 7200,
    ),
  ];

  final diagnostics = DiagnosticsSnapshot(
    capturedAtUnixMs: now * 1000,
    nodeIdentity: const NodeIdentitySnapshot(
      nodeId: 'shotnode0123456789abcdef',
      relayUrl: 'https://relay.example',
      localListenAddrs: ['192.168.1.20:11204'],
    ),
    peers: const [],
    connectionHistory: const [],
    errorLog: const [],
    gossipTopics: const [],
  );

  return [
    JynEvent.profile(profile: shotProfile(now: now)),
    JynEvent.friends(friends: friends, pending: pending),
    JynEvent.river(
      posts: posts,
      ghosts: ghosts,
      doors: const [],
      groupCards: const [],
    ),
    JynEvent.mediaReady(blobHash: 'shot-photo-blob', path: photoPath),
    JynEvent.mediaReady(blobHash: 'shot-audio-blob', path: audioPath),
    JynEvent.diagnostics(snapshot: diagnostics),
  ];
}
