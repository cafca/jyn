import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart' hide Visibility;

import '../actions.dart';
import '../rust/api/commands.dart' as commands;
import '../rust/api/lifecycle.dart' as lifecycle;
import '../rust/api/settings.dart' as rust;
import '../rust/settings.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import 'diagnostics_screen.dart';

/// Node settings. Relay and mDNS changes persist immediately and take
/// effect on the next app start.
class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  rust.SettingsView? _settings;
  final _relayUrl = TextEditingController();

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _relayUrl.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final settings = await rust.getSettings();
    if (!mounted) return;
    setState(() {
      _settings = settings;
      _relayUrl.text = settings.customRelayUrl ?? '';
    });
  }

  Future<void> _apply(Future<void> Function() change) async {
    await runGuarded(context, change);
    await _load();
  }

  Future<void> _showRecoveryPhrase() async {
    final String phrase;
    try {
      phrase = await lifecycle.recoveryPhrase();
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(error.toString())));
      }
      return;
    }
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('recovery phrase'),
        content: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 380),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Anyone with these words can be you. Write them down '
                'somewhere safe — together with a backup file they restore '
                'everything.',
                style: JynType.body.copyWith(
                  fontSize: 12.5,
                  color: JynColors.secondary,
                ),
              ),
              const SizedBox(height: 12),
              SelectableText(phrase, style: JynType.body),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('done'),
          ),
        ],
      ),
    );
  }

  Future<void> _exportBackup() async {
    final location = await getSaveLocation(
      suggestedName: 'jyn.backup',
      acceptedTypeGroups: const [
        XTypeGroup(label: 'jyn backup', extensions: ['backup']),
      ],
    );
    if (location == null || !mounted) return;
    await runGuarded(
      context,
      () => commands.exportBackup(destPath: location.path),
    );
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('backup written')),
    );
  }

  @override
  Widget build(BuildContext context) {
    final settings = _settings;
    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          const JynToolbar(showBack: true, title: 'Settings'),
          Expanded(child: _body(settings)),
        ],
      ),
    );
  }

  Widget _body(rust.SettingsView? settings) {
    // The list spans the window (scrollbar on the window edge); content
    // constrains itself to the 440px column.
    return settings == null
        ? const Center(child: CircularProgressIndicator())
        : ListView(
            padding: const EdgeInsets.symmetric(vertical: 16),
            children: [
              JynColumnItem(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'Changes take effect on the next app start.',
                      style: JynType.body.copyWith(
                        fontSize: 12.5,
                        color: JynColors.secondary,
                      ),
                    ),
                    const SizedBox(height: 8),
                    SwitchListTile(
                      title: const Text('local peer discovery (mDNS)'),
                      subtitle: const Text(
                        'find friends on the same network without a relay',
                      ),
                      value: settings.mdnsEnabled,
                      onChanged: (enabled) =>
                          _apply(() => rust.setMdnsEnabled(enabled: enabled)),
                    ),
                    const Divider(height: 32),
                    const Text('relay', style: JynType.name),
                    RadioGroup<RelayMode>(
                      groupValue: settings.relayMode,
                      onChanged: (mode) {
                        if (mode == null) return;
                        _apply(
                          () => rust.setRelayConfig(
                            relayMode: mode,
                            customRelayUrl: mode == RelayMode.relay
                                ? _relayUrl.text.trim()
                                : null,
                          ),
                        );
                      },
                      child: Column(
                        children: [
                          const RadioListTile<RelayMode>(
                            title: Text('testing relay (iroh EU)'),
                            value: RelayMode.testingRelay,
                          ),
                          const RadioListTile<RelayMode>(
                            title: Text('custom relay'),
                            value: RelayMode.relay,
                          ),
                          if (settings.relayMode == RelayMode.relay ||
                              _relayUrl.text.isNotEmpty)
                            Padding(
                              padding: const EdgeInsets.only(
                                left: 16,
                                bottom: 8,
                              ),
                              child: TextField(
                                controller: _relayUrl,
                                decoration: const InputDecoration(
                                  labelText: 'relay URL (https://…)',
                                  isDense: true,
                                  border: OutlineInputBorder(),
                                ),
                                onSubmitted: (url) => _apply(
                                  () => rust.setRelayConfig(
                                    relayMode: RelayMode.relay,
                                    customRelayUrl: url.trim(),
                                  ),
                                ),
                              ),
                            ),
                          const RadioListTile<RelayMode>(
                            title: Text('no relay (local network only)'),
                            value: RelayMode.disabled,
                          ),
                        ],
                      ),
                    ),
                    const Divider(height: 32),
                    const Text('backup', style: JynType.name),
                    ListTile(
                      contentPadding: EdgeInsets.zero,
                      leading: const Icon(
                        Icons.key_outlined,
                        color: JynColors.slate,
                      ),
                      title: const Text('recovery phrase'),
                      subtitle: const Text(
                        '24 words that recover your identity — keep them safe',
                      ),
                      trailing: const Icon(Icons.chevron_right),
                      onTap: _showRecoveryPhrase,
                    ),
                    ListTile(
                      contentPadding: EdgeInsets.zero,
                      leading: const Icon(
                        Icons.archive_outlined,
                        color: JynColors.slate,
                      ),
                      title: const Text('export backup…'),
                      subtitle: const Text(
                        'encrypted snapshot of your posts and keys; '
                        'restores with the recovery phrase',
                      ),
                      onTap: _exportBackup,
                    ),
                    const Divider(height: 32),
                    // Diagnostics moved off the top-level toolbar; it lives
                    // here now.
                    ListTile(
                      contentPadding: EdgeInsets.zero,
                      leading: const Icon(
                        Icons.monitor_heart_outlined,
                        color: JynColors.slate,
                      ),
                      title: const Text('diagnostics'),
                      subtitle: const Text(
                        'node identity, peers, gossip, history',
                      ),
                      trailing: const Icon(Icons.chevron_right),
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => const DiagnosticsScreen(),
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
