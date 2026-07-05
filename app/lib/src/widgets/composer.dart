import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';
import 'voice_recorder.dart';

/// The composer: body, visibility, lifetime, attachments, cast.
class Composer extends ConsumerStatefulWidget {
  const Composer({super.key});

  @override
  ConsumerState<Composer> createState() => _ComposerState();
}

class _ComposerState extends ConsumerState<Composer> {
  final _body = TextEditingController();
  Visibility? _visibility;
  int? _lifetimeSecs;
  bool _lifetimeTouched = false;
  bool _casting = false;
  final List<MediaDraftInput> _attachments = [];

  @override
  void dispose() {
    _body.dispose();
    super.dispose();
  }

  Future<void> _attachFiles() async {
    final files = await openFiles();
    if (files.isEmpty) return;
    setState(() {
      _attachments.addAll(files.map((f) => MediaDraftInput(path: f.path)));
    });
  }

  Future<void> _cast(Visibility visibility, int? lifetimeSecs) async {
    setState(() => _casting = true);
    await runGuarded(context, () async {
      await publishPost(
        body: _body.text.trim(),
        visibility: visibility,
        lifetimeSecs: lifetimeSecs,
        media: List.of(_attachments),
      );
      _body.clear();
      _attachments.clear();
    });
    if (mounted) setState(() => _casting = false);
  }

  @override
  Widget build(BuildContext context) {
    final profile = ref.watch(profileProvider);
    final visibility = _visibility ?? profile?.defaultVisibility ?? Visibility.friends;
    final lifetimeSecs =
        _lifetimeTouched ? _lifetimeSecs : profile?.defaultLifetimeSecs;
    final canCast = !_casting &&
        (_body.text.trim().isNotEmpty || _attachments.isNotEmpty);

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _body,
              minLines: 1,
              maxLines: 6,
              decoration: const InputDecoration(
                hintText: 'cast something into the river…',
                border: InputBorder.none,
              ),
              onChanged: (_) => setState(() {}),
            ),
            if (_attachments.isNotEmpty)
              Wrap(
                spacing: 8,
                runSpacing: 4,
                children: [
                  for (final (index, draft) in _attachments.indexed)
                    InputChip(
                      avatar: Icon(
                        draft.waveform != null
                            ? Icons.mic
                            : Icons.attach_file,
                        size: 16,
                      ),
                      label: Text(
                        draft.waveform != null
                            ? 'voice note'
                            : draft.path.split('/').last,
                        overflow: TextOverflow.ellipsis,
                      ),
                      onDeleted: () =>
                          setState(() => _attachments.removeAt(index)),
                    ),
                ],
              ),
            const SizedBox(height: 8),
            Row(
              children: [
                // Lifetime: the one property that matters.
                Expanded(
                  child: Wrap(
                    spacing: 4,
                    children: [
                      for (final (label, secs) in lifetimeOptions)
                        ChoiceChip(
                          label: Text(label),
                          selected: lifetimeSecs == secs,
                          visualDensity: VisualDensity.compact,
                          onSelected: (_) => setState(() {
                            _lifetimeTouched = true;
                            _lifetimeSecs = secs;
                          }),
                        ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                DropdownButton<Visibility>(
                  value: visibility,
                  underline: const SizedBox.shrink(),
                  items: [
                    for (final option in composerVisibilities)
                      DropdownMenuItem(
                        value: option,
                        child: Text(visibilityLabel(option)),
                      ),
                  ],
                  onChanged: (value) => setState(() => _visibility = value),
                ),
                const Spacer(),
                VoiceRecorderButton(
                  onRecorded: (draft) =>
                      setState(() => _attachments.add(draft)),
                ),
                IconButton(
                  tooltip: 'attach files',
                  onPressed: _attachFiles,
                  icon: const Icon(Icons.attach_file),
                ),
                FilledButton.icon(
                  onPressed:
                      canCast ? () => _cast(visibility, lifetimeSecs) : null,
                  icon: const Icon(Icons.water_drop_outlined, size: 18),
                  label: const Text('cast'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
