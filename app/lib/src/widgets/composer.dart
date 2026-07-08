import 'dart:ui';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../time_format.dart';
import 'jyn_avatar.dart';
import 'voice_note_player.dart';
import 'voice_recorder.dart';

/// The floating "cast" bar: a translucent pill pinned bottom-center that
/// grows in place into a compact card on focus — reach, lifetime scale,
/// attachments, cast. The feed stays visible behind it.
class Composer extends ConsumerStatefulWidget {
  const Composer({
    super.key,
    this.startExpanded = false,
    this.editOnly = false,
  });

  /// Boot straight into the expanded card (screenshot harness only).
  final bool startExpanded;

  /// Render nothing unless a post is being edited — screens without a
  /// composer of their own (the profile) still get the edit card.
  final bool editOnly;

  @override
  ConsumerState<Composer> createState() => _ComposerState();
}

class _ComposerState extends ConsumerState<Composer> {
  final _body = TextEditingController();
  final _focusNode = FocusNode();
  late bool _expanded = widget.startExpanded;
  Visibility _visibility = Visibility.circles;
  int? _lifetimeSecs = 24 * 3600;
  bool _casting = false;
  bool _reachMenuOpen = false;
  final List<MediaDraftInput> _attachments = [];

  @override
  void initState() {
    super.initState();
    _focusNode.addListener(() {
      if (_focusNode.hasFocus && !_expanded) setState(() => _expanded = true);
    });
  }

  @override
  void dispose() {
    _body.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  ReducedPost? get _editing => ref.read(editingPostProvider);

  void _expand() {
    setState(() => _expanded = true);
    _focusNode.requestFocus();
  }

  /// Collapse only when there's nothing at stake — an in-progress draft
  /// (or an edit in flight, or the open reach menu) keeps the card open.
  void _maybeCollapse() {
    if (_reachMenuOpen || _editing != null) return;
    if (_body.text.trim().isEmpty && _attachments.isEmpty) {
      _focusNode.unfocus();
      if (_expanded) setState(() => _expanded = false);
    }
  }

  Future<void> _attachFiles() async {
    final files = await openFiles();
    if (files.isEmpty) return;
    setState(() {
      _attachments.addAll(files.map((f) => MediaDraftInput(path: f.path)));
    });
  }

  /// The collapsed pill's paperclip: pick files, then open the card with
  /// them staged.
  Future<void> _attachFromPill() async {
    await _attachFiles();
    if (_attachments.isNotEmpty) _expand();
  }

  Future<void> _cast() async {
    setState(() => _casting = true);
    await runGuarded(context, () async {
      await publishPost(
        body: _body.text.trim(),
        visibility: _visibility,
        lifetimeSecs: _lifetimeSecs,
        media: List.of(_attachments),
      );
      _body.clear();
      _attachments.clear();
    });
    if (mounted) {
      setState(() => _casting = false);
      _maybeCollapse();
    }
  }

  bool get _canCast =>
      !_casting && (_body.text.trim().isNotEmpty || _attachments.isNotEmpty);

  // The lifetime chip that matched the post when editing started, so save
  // only touches the lifetime when the user actually moved it.
  int? _editInitialLifetime;

  /// The five-step chip that best represents a post's current lifetime.
  static int? _nearestLifetime(ReducedPost post) {
    final expiresAt = post.expiresAt;
    if (expiresAt == null) return null;
    final original = expiresAt - post.createdAt;
    int? best;
    var bestDiff = 1 << 62;
    for (final (_, secs) in ephemeralLifetimeOptions) {
      final diff = (secs - original).abs();
      if (diff < bestDiff) {
        bestDiff = diff;
        best = secs;
      }
    }
    return best;
  }

  /// Load a post into the card for editing (or clear back out).
  void _onEditingChanged(ReducedPost? previous, ReducedPost? next) {
    if (next != null && next.postId != previous?.postId) {
      _editInitialLifetime = _nearestLifetime(next);
      _body.text = next.body;
      setState(() {
        _visibility = next.visibility;
        _lifetimeSecs = _editInitialLifetime;
        _expanded = true;
      });
      _focusNode.requestFocus();
    }
  }

  /// Leave edit mode and restore the fresh-post defaults.
  void _cancelEdit() {
    ref.read(editingPostProvider.notifier).clear();
    _body.clear();
    _focusNode.unfocus();
    setState(() {
      _visibility = Visibility.circles;
      _lifetimeSecs = 24 * 3600;
      _expanded = false;
    });
  }

  /// Publish only what changed: the body (marked as edited) and/or the
  /// lifetime (a changed chip restarts the clock from now).
  Future<void> _saveEdit(ReducedPost editing) async {
    final body = _body.text.trim();
    if (body.isEmpty && editing.media.isEmpty) return;
    setState(() => _casting = true);
    await runGuarded(context, () async {
      if (body != editing.body) {
        await editPost(postId: editing.postId, body: body);
      }
      if (_lifetimeSecs != _editInitialLifetime) {
        await setPostLifetime(
          postId: editing.postId,
          expiresAt: _lifetimeSecs == null
              ? null
              : nowUnixSecs() + _lifetimeSecs!,
        );
      }
    });
    if (!mounted) return;
    setState(() => _casting = false);
    _cancelEdit();
  }

  Future<void> _confirmDelete(ReducedPost editing) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: const Text('delete post?'),
        content: const Text(
          'The delete reaches every copy, kept ones included.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('delete'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    await runGuarded(context, () => deletePost(postId: editing.postId));
    if (mounted) _cancelEdit();
  }

  @override
  Widget build(BuildContext context) {
    ref.listen<ReducedPost?>(editingPostProvider, _onEditingChanged);
    final editing = ref.watch(editingPostProvider);
    if (widget.editOnly && editing == null) return const SizedBox.shrink();

    final expanded = _expanded || editing != null;
    final radius = expanded ? JynRadii.sheet : JynRadii.pill;
    return TapRegion(
      onTapOutside: (_) => _maybeCollapse(),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 200),
        curve: Curves.easeOutCubic,
        width: expanded ? 452 : JynLayout.column,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(radius),
          boxShadow: expanded
              ? JynShadows.expandedCard
              : JynShadows.floatingPill,
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(radius),
          child: BackdropFilter(
            filter: ImageFilter.blur(
              sigmaX: expanded ? 14 : 12,
              sigmaY: expanded ? 14 : 12,
            ),
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 200),
              curve: Curves.easeOutCubic,
              decoration: BoxDecoration(
                color: expanded
                    ? JynColors.composerCardBg
                    : JynColors.composerPillBg,
                borderRadius: BorderRadius.circular(radius),
                border: Border.all(color: JynColors.hairline),
              ),
              child: AnimatedSize(
                duration: const Duration(milliseconds: 200),
                curve: Curves.easeOutCubic,
                alignment: Alignment.bottomCenter,
                child: expanded ? _card(editing) : _pill(),
              ),
            ),
          ),
        ),
      ),
    );
  }

  // ---- collapsed pill ---------------------------------------------------

  Widget _pill() {
    return MouseRegion(
      cursor: SystemMouseCursors.text,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: _expand,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 10),
          child: Row(
            children: [
              _selfAvatar(),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  'cast something into the river…',
                  style: JynType.body.copyWith(color: JynColors.muted),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              Tooltip(
                message: 'attach files',
                child: MouseRegion(
                  cursor: SystemMouseCursors.click,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: _attachFromPill,
                    child: const Padding(
                      padding: EdgeInsets.all(4),
                      child: Icon(
                        Icons.attach_file,
                        size: 18,
                        color: JynColors.slate,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  // ---- expanded card ------------------------------------------------------

  /// The one composer card, for casting and editing alike. In edit mode
  /// the reach is fixed, attachments are frozen, and cast becomes
  /// delete + save.
  Widget _card(ReducedPost? editing) {
    final isEditing = editing != null;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 13, 16, 13),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              _selfAvatar(),
              const SizedBox(width: 8),
              Text(
                isEditing ? 'editing your post' : 'to your river',
                style: JynType.body.copyWith(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
              const Spacer(),
              _reachPill(interactive: !isEditing),
              if (isEditing) ...[
                const SizedBox(width: 8),
                Tooltip(
                  message: 'discard changes',
                  child: MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: GestureDetector(
                      onTap: _cancelEdit,
                      child: const Icon(
                        Icons.close,
                        size: 17,
                        color: JynColors.secondary,
                      ),
                    ),
                  ),
                ),
              ],
            ],
          ),
          const SizedBox(height: 8),
          TextField(
            controller: _body,
            focusNode: _focusNode,
            minLines: 1,
            maxLines: 6,
            style: JynType.body.copyWith(fontSize: 15),
            cursorColor: JynColors.leaf,
            cursorWidth: 2,
            decoration: InputDecoration(
              hintText: isEditing ? null : 'cast something into the river…',
              hintStyle: JynType.body.copyWith(
                fontSize: 15,
                color: JynColors.muted,
              ),
              isDense: true,
              border: InputBorder.none,
            ),
            // The outer TapRegion decides when to collapse; don't let the
            // field unfocus itself when card buttons are clicked.
            onTapOutside: (_) {},
            onChanged: (_) => setState(() {}),
          ),
          if (!isEditing && _attachments.isNotEmpty) ...[
            const SizedBox(height: 8),
            _attachmentList(),
          ],
          const SizedBox(height: 10),
          Container(height: 1, color: JynColors.hairlineFaint),
          const SizedBox(height: 10),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final (label, secs) in lifetimeOptions)
                _lifetimeChip(label, secs),
            ],
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              if (isEditing)
                // Attachments travel with the original cast; freeze them.
                Upcoming(
                  message: 'attachments can’t be edited yet',
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      _attachButton(),
                      const SizedBox(width: 4),
                      const Icon(
                        Icons.mic_none,
                        size: 22,
                        color: JynColors.slate,
                      ),
                    ],
                  ),
                )
              else ...[
                _attachButton(),
                const SizedBox(width: 4),
                VoiceRecorderButton(
                  onRecorded: (draft) =>
                      setState(() => _attachments.add(draft)),
                ),
              ],
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  isEditing
                      ? 'saved edits are marked'
                      : 'attach a photo, audio or file',
                  style: JynType.body.copyWith(
                    fontSize: 12,
                    color: JynColors.muted,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (isEditing) ...[
                _deleteButton(editing),
                const SizedBox(width: 8),
                _primaryPill(
                  label: 'save',
                  enabled:
                      !_casting &&
                      (_body.text.trim().isNotEmpty ||
                          editing.media.isNotEmpty),
                  onTap: () => _saveEdit(editing),
                ),
              ] else
                _castButton(),
            ],
          ),
        ],
      ),
    );
  }

  Widget _deleteButton(ReducedPost editing) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: () => _confirmDelete(editing),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(JynRadii.button),
            border: Border.all(color: const Color(0x33B3402A)),
          ),
          child: const Text(
            'delete',
            style: TextStyle(
              fontSize: 13,
              fontWeight: FontWeight.w600,
              color: Color(0xFFB3402A),
              height: 1.0,
            ),
          ),
        ),
      ),
    );
  }

  Widget _selfAvatar() {
    final profile = ref.watch(profileProvider);
    return JynAvatar(
      profileId: profile?.profileId ?? '',
      displayName: profile?.displayName ?? '',
      size: 30,
      isSelf: true,
    );
  }

  Widget _reachPill({required bool interactive}) {
    final pill = Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: JynColors.chipTint,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        interactive
            ? '${visibilityLabel(_visibility)} ▾'
            : visibilityLabel(_visibility),
        style: JynType.body.copyWith(fontSize: 12, color: JynColors.mid),
      ),
    );
    // Reach can't change after casting; in edit mode the pill is a label.
    if (!interactive) return pill;
    return MenuAnchor(
      // The menu overlay counts as "outside" the card's TapRegion; flag it
      // open so picking a reach never collapses the composer.
      onOpen: () => _reachMenuOpen = true,
      onClose: () {
        _reachMenuOpen = false;
        _focusNode.requestFocus();
      },
      builder: (context, controller, _) => MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          onTap: () =>
              controller.isOpen ? controller.close() : controller.open(),
          child: pill,
        ),
      ),
      menuChildren: [
        for (final option in composerVisibilities)
          MenuItemButton(
            onPressed: () => setState(() => _visibility = option),
            child: Text(visibilityLabel(option)),
          ),
      ],
    );
  }

  Widget _lifetimeChip(String label, int? secs) {
    final selected = _lifetimeSecs == secs;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: () => setState(() => _lifetimeSecs = secs),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
          decoration: BoxDecoration(
            color: selected ? JynColors.chipSelected : JynColors.cardGrey,
            borderRadius: BorderRadius.circular(999),
            border: Border.all(
              color: selected
                  ? JynColors.chipSelectedBorder
                  : Colors.transparent,
            ),
          ),
          child: Text(
            selected ? '✓ $label' : label,
            style: TextStyle(
              fontFamily: JynType.mono,
              fontSize: 11.5,
              color: selected ? JynColors.ink : JynColors.slate,
              height: 1.0,
            ),
          ),
        ),
      ),
    );
  }

  Widget _attachButton() {
    return Tooltip(
      message: 'attach files',
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          onTap: _attachFiles,
          child: Container(
            width: 36,
            height: 36,
            decoration: BoxDecoration(
              color: JynColors.field,
              borderRadius: BorderRadius.circular(JynRadii.attach),
            ),
            child: const Icon(
              Icons.attach_file,
              size: 18,
              color: JynColors.slate,
            ),
          ),
        ),
      ),
    );
  }

  Widget _castButton() =>
      _primaryPill(label: 'cast', enabled: _canCast, onTap: _cast, leaf: true);

  Widget _primaryPill({
    required String label,
    required bool enabled,
    required VoidCallback onTap,
    bool leaf = false,
  }) {
    return MouseRegion(
      cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
      child: GestureDetector(
        onTap: enabled ? onTap : null,
        child: AnimatedOpacity(
          duration: const Duration(milliseconds: 120),
          opacity: enabled ? 1 : 0.45,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 9),
            decoration: BoxDecoration(
              color: JynColors.leaf,
              borderRadius: BorderRadius.circular(JynRadii.button),
              boxShadow: enabled ? JynShadows.primaryButton : null,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (leaf) ...[
                  const Icon(Icons.eco, size: 15, color: Colors.white),
                  const SizedBox(width: 6),
                ],
                Text(
                  label,
                  style: const TextStyle(
                    fontSize: 13.5,
                    fontWeight: FontWeight.w600,
                    color: Colors.white,
                    height: 1.0,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _attachmentList() {
    return Wrap(
      spacing: 8,
      runSpacing: 6,
      children: [
        for (final (index, draft) in _attachments.indexed)
          if (draft.waveform != null)
            // Recorded voice notes are always WAV; play them back before
            // casting, with a compact remove affordance.
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  width: 320,
                  child: VoiceNotePlayer(
                    waveform: draft.waveform,
                    durationMs: draft.durationMs,
                    mime: 'audio/wav',
                    path: draft.path,
                  ),
                ),
                IconButton(
                  visualDensity: VisualDensity.compact,
                  tooltip: 'remove voice note',
                  icon: const Icon(Icons.close, size: 18),
                  onPressed: () => setState(() => _attachments.removeAt(index)),
                ),
              ],
            )
          else
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              decoration: BoxDecoration(
                color: JynColors.field,
                borderRadius: BorderRadius.circular(JynRadii.attach),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Icon(
                    Icons.attach_file,
                    size: 14,
                    color: JynColors.slate,
                  ),
                  const SizedBox(width: 6),
                  ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 180),
                    child: Text(
                      draft.path.split('/').last,
                      overflow: TextOverflow.ellipsis,
                      style: JynType.body.copyWith(fontSize: 12.5),
                    ),
                  ),
                  const SizedBox(width: 4),
                  MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: GestureDetector(
                      onTap: () => setState(() => _attachments.removeAt(index)),
                      child: const Icon(
                        Icons.close,
                        size: 14,
                        color: JynColors.secondary,
                      ),
                    ),
                  ),
                ],
              ),
            ),
      ],
    );
  }
}
