import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/api/lifecycle.dart' as rust;
import '../rust/domain.dart';
import '../rust/profile.dart';
import '../rust/runtime.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/composer.dart';
import '../widgets/jyn_avatar.dart';
import '../widgets/post_card.dart';
import 'settings_screen.dart';

/// A sheet the profile can boot straight into (screenshot harness only).
enum ProfileSheet { none, addFriend, editProfile }

/// The stripped-back profile (`7b`): identity header, the friends area
/// (avatars, incoming requests, the Add-a-friend sheet), then your own
/// stream with its honest lifetime states.
class ProfileScreen extends ConsumerStatefulWidget {
  const ProfileScreen({super.key, this.initialSheet = ProfileSheet.none});

  final ProfileSheet initialSheet;

  @override
  ConsumerState<ProfileScreen> createState() => _ProfileScreenState();
}

class _ProfileScreenState extends ConsumerState<ProfileScreen> {
  @override
  void initState() {
    super.initState();
    if (widget.initialSheet == ProfileSheet.none) return;
    // Give the fixture events a beat to land before opening the sheet.
    Future<void>.delayed(const Duration(milliseconds: 400), () {
      if (!mounted) return;
      switch (widget.initialSheet) {
        case ProfileSheet.addFriend:
          showDialog<void>(
            context: context,
            builder: (_) => const _AddFriendSheet(),
          );
        case ProfileSheet.editProfile:
          final profile = ref.read(profileProvider);
          if (profile != null) {
            showDialog<void>(
              context: context,
              builder: (_) => _EditProfileSheet(profile: profile),
            );
          }
        case ProfileSheet.none:
          break;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final profile = ref.watch(profileProvider);
    final friends = ref.watch(friendsProvider);
    final pending = ref.watch(pendingRequestsProvider);
    final ownPosts = ref
        .watch(riverPostsProvider)
        .where((p) => p.isSelf)
        .toList();

    if (profile == null) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          JynToolbar(
            showBack: true,
            title: 'Profile',
            actions: [
              const JynSearchField(width: 160),
              const SizedBox(width: 12),
              JynToolbarIcon(
                icon: Icons.settings_outlined,
                tooltip: 'settings',
                onTap: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => const SettingsScreen(),
                  ),
                ),
              ),
            ],
          ),
          Expanded(
            child: Stack(
              children: [
                // Full-width list; items constrain to the 440px column.
                Positioned.fill(
                  child: ListView(
                    padding: const EdgeInsets.symmetric(vertical: 22),
                    children: [
                      JynColumnItem(child: _IdentityHeader(profile: profile)),
                      const SizedBox(height: 26),
                      JynColumnItem(
                        child: _FriendsArea(friends: friends, pending: pending),
                      ),
                      const SizedBox(height: 22),
                      const JynColumnItem(child: JynHairline(faint: true)),
                      if (ownPosts.isEmpty)
                        JynColumnItem(
                          child: Padding(
                            padding: const EdgeInsets.symmetric(vertical: 40),
                            child: Center(
                              child: Text(
                                'nothing cast yet',
                                style: JynType.body.copyWith(
                                  color: JynColors.muted,
                                ),
                              ),
                            ),
                          ),
                        ),
                      for (final (index, post) in ownPosts.indexed) ...[
                        if (index > 0)
                          const JynColumnItem(child: JynHairline(faint: true)),
                        JynColumnItem(child: PostCard(post: post)),
                      ],
                    ],
                  ),
                ),
                // Editing a post from the stream brings up the composer's
                // edit card here too.
                const Positioned(
                  left: 0,
                  right: 0,
                  bottom: 18,
                  child: Center(child: Composer(editOnly: true)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ---- identity --------------------------------------------------------------

class _IdentityHeader extends StatelessWidget {
  const _IdentityHeader({required this.profile});

  final UserProfile profile;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        JynAvatar(
          profileId: profile.profileId,
          displayName: profile.displayName,
          size: 70,
          isSelf: true,
        ),
        const SizedBox(width: 16),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const SizedBox(height: 6),
              Row(
                children: [
                  Flexible(
                    child: Text(
                      profile.displayName,
                      style: const TextStyle(
                        fontSize: 20,
                        fontWeight: FontWeight.w700,
                        color: JynColors.ink,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: 10),
                  _EditChip(profile: profile),
                ],
              ),
              if (profile.bio.isNotEmpty) ...[
                const SizedBox(height: 4),
                Text(
                  profile.bio,
                  style: JynType.body.copyWith(
                    fontSize: 13,
                    color: JynColors.textSoft,
                  ),
                ),
              ],
            ],
          ),
        ),
      ],
    );
  }
}

class _EditChip extends StatelessWidget {
  const _EditChip({required this.profile});

  final UserProfile profile;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: () => showDialog<void>(
          context: context,
          builder: (_) => _EditProfileSheet(profile: profile),
        ),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
          decoration: BoxDecoration(
            color: JynColors.field,
            borderRadius: BorderRadius.circular(999),
          ),
          child: Text(
            'Edit',
            style: JynType.body.copyWith(fontSize: 12, color: JynColors.slate),
          ),
        ),
      ),
    );
  }
}

/// Edit sheet: name and bio only (composer defaults are no longer settable
/// here; the profile image is upcoming).
class _EditProfileSheet extends StatefulWidget {
  const _EditProfileSheet({required this.profile});

  final UserProfile profile;

  @override
  State<_EditProfileSheet> createState() => _EditProfileSheetState();
}

class _EditProfileSheetState extends State<_EditProfileSheet> {
  late final _name = TextEditingController(text: widget.profile.displayName);
  late final _bio = TextEditingController(text: widget.profile.bio);
  bool _saving = false;

  @override
  void dispose() {
    _name.dispose();
    _bio.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    setState(() => _saving = true);
    await runGuarded(context, () async {
      await updateProfile(
        displayName: _name.text.trim(),
        bio: _bio.text.trim(),
        // Defaults ride along unchanged — they're not user-settable for now.
        defaultVisibility: widget.profile.defaultVisibility,
        defaultLifetimeSecs: widget.profile.defaultLifetimeSecs,
        markOnboarded: false,
      );
    });
    if (mounted) {
      setState(() => _saving = false);
      Navigator.of(context).pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    return _SheetShell(
      title: 'Edit profile',
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              JynAvatar(
                profileId: widget.profile.profileId,
                displayName: _name.text,
                size: 54,
                isSelf: true,
              ),
              const SizedBox(width: 12),
              Upcoming(
                message: 'profile images are coming soon',
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 10,
                    vertical: 5,
                  ),
                  decoration: BoxDecoration(
                    color: JynColors.field,
                    borderRadius: BorderRadius.circular(999),
                  ),
                  child: Text(
                    'change photo',
                    style: JynType.body.copyWith(
                      fontSize: 12,
                      color: JynColors.slate,
                    ),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 16),
          _field('name', _name, minLines: 1, maxLines: 1),
          const SizedBox(height: 12),
          _field('bio', _bio, minLines: 2, maxLines: 4),
          const SizedBox(height: 18),
          Align(
            alignment: Alignment.centerRight,
            child: _PrimaryButton(
              label: 'save',
              enabled: !_saving,
              onTap: _save,
            ),
          ),
        ],
      ),
    );
  }

  Widget _field(
    String label,
    TextEditingController controller, {
    required int minLines,
    required int maxLines,
  }) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label, style: JynType.meta),
        const SizedBox(height: 4),
        TextField(
          controller: controller,
          minLines: minLines,
          maxLines: maxLines,
          style: JynType.body.copyWith(fontSize: 14),
          cursorColor: JynColors.leaf,
          onChanged: (_) => setState(() {}),
          decoration: InputDecoration(
            isDense: true,
            filled: true,
            fillColor: JynColors.field,
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 10,
              vertical: 9,
            ),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(JynRadii.chip),
              borderSide: BorderSide.none,
            ),
          ),
        ),
      ],
    );
  }
}

// ---- friends -----------------------------------------------------------------

class _FriendsArea extends StatelessWidget {
  const _FriendsArea({required this.friends, required this.pending});

  final List<FriendEntry> friends;
  final List<PendingFriendRequest> pending;

  static const _maxAvatars = 6;

  @override
  Widget build(BuildContext context) {
    final overflow = friends.length - _maxAvatars;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            const Text('Friends', style: JynType.name),
            const SizedBox(width: 6),
            Text('${friends.length}', style: JynType.meta),
            const Spacer(),
            MouseRegion(
              cursor: SystemMouseCursors.click,
              child: GestureDetector(
                onTap: () => showDialog<void>(
                  context: context,
                  builder: (_) => const _AddFriendSheet(),
                ),
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 5,
                  ),
                  decoration: BoxDecoration(
                    color: JynColors.chipSelected,
                    borderRadius: BorderRadius.circular(999),
                  ),
                  child: Text(
                    '+ Add',
                    style: JynType.body.copyWith(
                      fontSize: 12.5,
                      fontWeight: FontWeight.w600,
                      color: JynColors.ink,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        if (friends.isEmpty)
          Text(
            'nobody yet — trade codes with a friend',
            style: JynType.body.copyWith(fontSize: 13, color: JynColors.muted),
          )
        else
          Wrap(
            spacing: 8,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              for (final friend in friends.take(_maxAvatars))
                _FriendAvatar(friend: friend),
              if (overflow > 0)
                Container(
                  width: 42,
                  height: 42,
                  alignment: Alignment.center,
                  decoration: const BoxDecoration(
                    shape: BoxShape.circle,
                    color: JynColors.field,
                  ),
                  child: Text(
                    '+$overflow',
                    style: JynType.body.copyWith(
                      fontSize: 12.5,
                      color: JynColors.secondary,
                    ),
                  ),
                ),
            ],
          ),
        for (final request in pending) ...[
          const SizedBox(height: 12),
          _RequestCard(request: request),
        ],
      ],
    );
  }
}

/// A friend avatar; clicking opens their profile (which carries the
/// friendship state and the unfriend action).
class _FriendAvatar extends ConsumerWidget {
  const _FriendAvatar({required this.friend});

  final FriendEntry friend;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Tooltip(
      message: friend.followsMeBack
          ? friend.displayName
          : '${friend.displayName} — awaiting their answer',
      child: Opacity(
        opacity: friend.followsMeBack ? 1 : 0.55,
        child: JynAvatar(
          profileId: friend.profileId,
          displayName: friend.displayName,
          size: 42,
          onTap: () => openUserProfile(
            context,
            ref,
            profileId: friend.profileId,
            displayName: friend.displayName,
          ),
        ),
      ),
    );
  }
}

/// An incoming request: the highlighted card in the friends area.
class _RequestCard extends StatelessWidget {
  const _RequestCard({required this.request});

  final PendingFriendRequest request;

  @override
  Widget build(BuildContext context) {
    final greeting = request.greeting;
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: JynColors.requestBg,
        borderRadius: BorderRadius.circular(JynRadii.media),
        border: Border.all(color: JynColors.requestBorder),
      ),
      child: Row(
        children: [
          JynAvatar(
            profileId: request.requesterProfileId,
            displayName: request.requesterDisplayName,
            size: 34,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  request.requesterDisplayName,
                  style: JynType.body.copyWith(
                    fontSize: 13.5,
                    fontWeight: FontWeight.w600,
                    color: JynColors.requestName,
                  ),
                ),
                Text(
                  greeting == null || greeting.isEmpty
                      ? 'wants to add you as a friend'
                      : '“$greeting”',
                  style: JynType.body.copyWith(
                    fontSize: 12,
                    color: JynColors.requestText,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          _pillButton(
            context,
            label: 'Accept',
            background: JynColors.accept,
            foreground: Colors.white,
            onTap: () => runGuarded(
              context,
              () => respondFriendship(
                requesterProfileId: request.requesterProfileId,
                accept: true,
              ),
            ),
          ),
          const SizedBox(width: 6),
          _pillButton(
            context,
            label: 'Ignore',
            background: JynColors.ignoreBg,
            foreground: JynColors.requestText,
            onTap: () => runGuarded(
              context,
              () => respondFriendship(
                requesterProfileId: request.requesterProfileId,
                accept: false,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _pillButton(
    BuildContext context, {
    required String label,
    required Color background,
    required Color foreground,
    required VoidCallback onTap,
  }) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
          decoration: BoxDecoration(
            color: background,
            borderRadius: BorderRadius.circular(999),
          ),
          child: Text(
            label,
            style: TextStyle(
              fontSize: 12.5,
              fontWeight: FontWeight.w600,
              color: foreground,
            ),
          ),
        ),
      ),
    );
  }
}

// ---- add-a-friend sheet --------------------------------------------------------

class _AddFriendSheet extends StatefulWidget {
  const _AddFriendSheet();

  @override
  State<_AddFriendSheet> createState() => _AddFriendSheetState();
}

class _AddFriendSheetState extends State<_AddFriendSheet> {
  final _codeInput = TextEditingController();

  @override
  void dispose() {
    _codeInput.dispose();
    super.dispose();
  }

  bool get _codeValid => _codeInput.text.trim().startsWith('jyn-');

  Future<void> _request() async {
    final code = _codeInput.text.trim();
    if (!code.startsWith('jyn-')) return;
    await runGuarded(context, () => requestFriendship(friendCode: code));
    _codeInput.clear();
    if (mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    return _SheetShell(
      title: 'Add a friend',
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'There are no usernames on jyn. You connect by trading a code — '
            'hand yours out, or enter one you were given.',
            style: JynType.body.copyWith(
              fontSize: 12.5,
              color: JynColors.textSoft,
            ),
          ),
          const SizedBox(height: 14),
          // PRIMARY — your code.
          Container(
            width: double.infinity,
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              gradient: const LinearGradient(
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
                colors: [JynColors.sheetCardTop, JynColors.sheetCardBottom],
              ),
              borderRadius: BorderRadius.circular(JynRadii.card),
              border: Border.all(color: JynColors.sheetCardBorder),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text('YOUR CODE', style: JynType.capsLabel),
                const SizedBox(height: 8),
                FutureBuilder<String>(
                  future: rust.myFriendCode(),
                  builder: (context, snapshot) {
                    final code = snapshot.data;
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        SelectableText(
                          code ?? '…',
                          style: JynType.shareCode.copyWith(fontSize: 13),
                        ),
                        const SizedBox(height: 6),
                        Text(
                          'Hand this to someone and they can add you.',
                          style: JynType.body.copyWith(
                            fontSize: 12,
                            color: JynColors.mid,
                          ),
                        ),
                        const SizedBox(height: 12),
                        SizedBox(
                          width: double.infinity,
                          child: _PrimaryButton(
                            label: 'Copy code',
                            icon: Icons.copy,
                            enabled: code != null,
                            onTap: () async {
                              await Clipboard.setData(
                                ClipboardData(text: code!),
                              );
                              if (context.mounted) {
                                ScaffoldMessenger.of(context).showSnackBar(
                                  const SnackBar(content: Text('code copied')),
                                );
                              }
                            },
                          ),
                        ),
                      ],
                    );
                  },
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          // SECONDARY — enter a code.
          Text(
            'Got someone’s code?',
            style: JynType.body.copyWith(fontSize: 12, color: JynColors.muted),
          ),
          const SizedBox(height: 6),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _codeInput,
                  style: const TextStyle(
                    fontFamily: JynType.mono,
                    fontSize: 12.5,
                    color: JynColors.text,
                  ),
                  cursorColor: JynColors.leaf,
                  decoration: InputDecoration(
                    hintText: 'enter a code…',
                    hintStyle: const TextStyle(
                      fontFamily: JynType.mono,
                      fontSize: 12.5,
                      color: JynColors.muted,
                    ),
                    isDense: true,
                    contentPadding: const EdgeInsets.symmetric(
                      horizontal: 10,
                      vertical: 9,
                    ),
                    enabledBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(JynRadii.chip),
                      borderSide: const BorderSide(
                        color: JynColors.chipOutline,
                      ),
                    ),
                    focusedBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(JynRadii.chip),
                      borderSide: const BorderSide(color: JynColors.mid),
                    ),
                  ),
                  onChanged: (_) => setState(() {}),
                  onSubmitted: (_) => _request(),
                ),
              ),
              const SizedBox(width: 8),
              MouseRegion(
                cursor: _codeValid
                    ? SystemMouseCursors.click
                    : SystemMouseCursors.basic,
                child: GestureDetector(
                  onTap: _codeValid ? _request : null,
                  child: Opacity(
                    opacity: _codeValid ? 1 : 0.5,
                    child: Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 14,
                        vertical: 8,
                      ),
                      decoration: BoxDecoration(
                        borderRadius: BorderRadius.circular(JynRadii.chip),
                        border: Border.all(color: JynColors.chipOutline),
                      ),
                      child: Text(
                        'Add',
                        style: JynType.body.copyWith(
                          fontSize: 13,
                          fontWeight: FontWeight.w600,
                          color: JynColors.mid,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

// ---- shared sheet scaffolding ----------------------------------------------------

/// A centered 392px white card, radius 22 — the design's sheet.
class _SheetShell extends StatelessWidget {
  const _SheetShell({required this.title, required this.child});

  final String title;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Dialog(
      backgroundColor: Colors.transparent,
      child: Container(
        width: 392,
        padding: const EdgeInsets.all(20),
        decoration: BoxDecoration(
          color: Colors.white,
          borderRadius: BorderRadius.circular(JynRadii.sheet),
          boxShadow: JynShadows.expandedCard,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text(
                  title,
                  style: const TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.w700,
                    color: JynColors.ink,
                  ),
                ),
                const Spacer(),
                MouseRegion(
                  cursor: SystemMouseCursors.click,
                  child: GestureDetector(
                    onTap: () => Navigator.of(context).pop(),
                    child: const Icon(
                      Icons.close,
                      size: 18,
                      color: JynColors.secondary,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 12),
            child,
          ],
        ),
      ),
    );
  }
}

/// The filled brand-green button (save/copy).
class _PrimaryButton extends StatelessWidget {
  const _PrimaryButton({
    required this.label,
    required this.onTap,
    this.icon,
    this.enabled = true,
  });

  final String label;
  final VoidCallback onTap;
  final IconData? icon;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
      child: GestureDetector(
        onTap: enabled ? onTap : null,
        child: Opacity(
          opacity: enabled ? 1 : 0.5,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 9),
            decoration: BoxDecoration(
              color: JynColors.leaf,
              borderRadius: BorderRadius.circular(JynRadii.button),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                if (icon != null) ...[
                  Icon(icon, size: 15, color: Colors.white),
                  const SizedBox(width: 6),
                ],
                Text(
                  label,
                  style: const TextStyle(
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                    color: Colors.white,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
