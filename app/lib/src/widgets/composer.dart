import 'dart:ui';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';
import '../theme/tokens.dart';
import 'jyn_avatar.dart';
import 'voice_note_player.dart';
import 'voice_recorder.dart';

/// The floating "cast" bar: a translucent pill pinned bottom-center that
/// grows in place into a compact card on focus — reach, lifetime scale,
/// attachments, cast. The feed stays visible behind it.
class Composer extends ConsumerStatefulWidget {
  const Composer({super.key, this.startExpanded = false});

  /// Boot straight into the expanded card (screenshot harness only).
  final bool startExpanded;

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

  void _expand() {
    setState(() => _expanded = true);
    _focusNode.requestFocus();
  }

  /// Collapse only when there's nothing at stake — an in-progress draft
  /// keeps the card open.
  void _maybeCollapse() {
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

  @override
  Widget build(BuildContext context) {
    final radius = _expanded ? JynRadii.sheet : JynRadii.pill;
    return TapRegion(
      onTapOutside: (_) => _maybeCollapse(),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 200),
        curve: Curves.easeOutCubic,
        width: _expanded ? 452 : JynLayout.column,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(radius),
          boxShadow: _expanded
              ? JynShadows.expandedCard
              : JynShadows.floatingPill,
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(radius),
          child: BackdropFilter(
            filter: ImageFilter.blur(
              sigmaX: _expanded ? 14 : 12,
              sigmaY: _expanded ? 14 : 12,
            ),
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 200),
              curve: Curves.easeOutCubic,
              decoration: BoxDecoration(
                color: _expanded
                    ? JynColors.composerCardBg
                    : JynColors.composerPillBg,
                borderRadius: BorderRadius.circular(radius),
                border: Border.all(color: JynColors.hairline),
              ),
              child: AnimatedSize(
                duration: const Duration(milliseconds: 200),
                curve: Curves.easeOutCubic,
                alignment: Alignment.bottomCenter,
                child: _expanded ? _card() : _pill(),
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
              _reachPill(interactive: false),
              const SizedBox(width: 8),
              const Icon(Icons.attach_file, size: 18, color: JynColors.slate),
            ],
          ),
        ),
      ),
    );
  }

  // ---- expanded card ------------------------------------------------------

  Widget _card() {
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
                'to your river',
                style: JynType.body.copyWith(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
              const Spacer(),
              _reachPill(interactive: true),
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
              hintText: 'cast something into the river…',
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
          if (_attachments.isNotEmpty) ...[
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
              _attachButton(),
              const SizedBox(width: 4),
              VoiceRecorderButton(
                onRecorded: (draft) => setState(() => _attachments.add(draft)),
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  'attach a photo, audio or file',
                  style: JynType.body.copyWith(
                    fontSize: 12,
                    color: JynColors.muted,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              _castButton(),
            ],
          ),
        ],
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
    final label = Text(
      interactive
          ? '${visibilityLabel(_visibility)} ▾'
          : visibilityLabel(_visibility),
      style: JynType.body.copyWith(fontSize: 12, color: JynColors.mid),
    );
    final pill = Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: JynColors.chipTint,
        borderRadius: BorderRadius.circular(999),
      ),
      child: label,
    );
    if (!interactive) return pill;
    return MenuAnchor(
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

  Widget _castButton() {
    final enabled = _canCast;
    return MouseRegion(
      cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
      child: GestureDetector(
        onTap: enabled ? _cast : null,
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
              children: const [
                Icon(Icons.eco, size: 15, color: Colors.white),
                SizedBox(width: 6),
                Text(
                  'cast',
                  style: TextStyle(
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
