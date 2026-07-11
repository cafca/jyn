import 'dart:async';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';

import '../rust/api/lifecycle.dart' as lifecycle;
import '../theme/tokens.dart';

/// Pre-start chooser on a fresh machine: begin new, or restore a backup.
///
/// Runs as its own `runApp` before the node starts, because a restore must
/// land before the stores are opened. Completes when the user starts fresh
/// or a restore succeeded; `main` then calls `startNode` and boots the real
/// app on whatever the choice left in the data directory.
Future<void> runRestoreGate() async {
  final completer = Completer<void>();
  runApp(
    MaterialApp(
      title: 'jyn',
      theme: jynTheme(),
      home: _RestoreGate(onDone: completer.complete),
    ),
  );
  await completer.future;
}

class _RestoreGate extends StatefulWidget {
  const _RestoreGate({required this.onDone});

  final VoidCallback onDone;

  @override
  State<_RestoreGate> createState() => _RestoreGateState();
}

class _RestoreGateState extends State<_RestoreGate> {
  final _phrase = TextEditingController();
  String? _archivePath;
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _phrase.dispose();
    super.dispose();
  }

  Future<void> _pickArchive() async {
    final file = await openFile(
      acceptedTypeGroups: const [
        XTypeGroup(label: 'jyn backup', extensions: ['backup']),
      ],
    );
    if (file == null) return;
    setState(() => _archivePath = file.path);
  }

  Future<void> _restore() async {
    final archive = _archivePath;
    if (archive == null || _phrase.text.trim().isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await lifecycle.restoreBackup(
        archivePath: archive,
        recoveryPhrase: _phrase.text,
      );
      widget.onDone();
    } catch (error) {
      setState(() {
        _busy = false;
        _error = error.toString();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 440),
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text('jyn', style: JynType.name.copyWith(fontSize: 28)),
                const SizedBox(height: 8),
                Text(
                  'Coming back? Restore a backup with your recovery '
                  'phrase — or begin anew.',
                  style: JynType.body.copyWith(color: JynColors.secondary),
                ),
                const SizedBox(height: 24),
                OutlinedButton.icon(
                  onPressed: _busy ? null : _pickArchive,
                  icon: const Icon(Icons.archive_outlined),
                  label: Text(
                    _archivePath == null
                        ? 'choose backup file…'
                        : _archivePath!.split('/').last,
                  ),
                ),
                if (_archivePath != null) ...[
                  const SizedBox(height: 12),
                  TextField(
                    controller: _phrase,
                    enabled: !_busy,
                    minLines: 2,
                    maxLines: 3,
                    decoration: const InputDecoration(
                      labelText: 'recovery phrase (24 words)',
                      border: OutlineInputBorder(),
                    ),
                    onSubmitted: (_) => _restore(),
                  ),
                  const SizedBox(height: 12),
                  FilledButton(
                    onPressed: _busy ? null : _restore,
                    child: _busy
                        ? const SizedBox(
                            width: 18,
                            height: 18,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Text('restore'),
                  ),
                ],
                if (_error != null) ...[
                  const SizedBox(height: 12),
                  Text(
                    _error!,
                    style: JynType.body.copyWith(color: Colors.red.shade700),
                  ),
                ],
                const SizedBox(height: 24),
                TextButton(
                  onPressed: _busy ? null : widget.onDone,
                  child: const Text('begin anew'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
