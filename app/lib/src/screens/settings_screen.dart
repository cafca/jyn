import 'package:flutter/material.dart' hide Visibility;

import '../actions.dart';
import '../rust/api/settings.dart' as rust;
import '../rust/settings.dart';

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

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final settings = _settings;
    return Scaffold(
      appBar: AppBar(title: const Text('settings')),
      body: settings == null
          ? const Center(child: CircularProgressIndicator())
          : Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 640),
                child: ListView(
                  padding: const EdgeInsets.all(16),
                  children: [
                    Text(
                      'Changes take effect on the next app start.',
                      style: theme.textTheme.bodySmall
                          ?.copyWith(color: theme.colorScheme.outline),
                    ),
                    const SizedBox(height: 8),
                    SwitchListTile(
                      title: const Text('local peer discovery (mDNS)'),
                      subtitle: const Text(
                          'find friends on the same network without a relay'),
                      value: settings.mdnsEnabled,
                      onChanged: (enabled) =>
                          _apply(() => rust.setMdnsEnabled(enabled: enabled)),
                    ),
                    const Divider(height: 32),
                    Text('relay', style: theme.textTheme.titleSmall),
                    RadioGroup<RelayMode>(
                      groupValue: settings.relayMode,
                      onChanged: (mode) {
                        if (mode == null) return;
                        _apply(() => rust.setRelayConfig(
                              relayMode: mode,
                              customRelayUrl: mode == RelayMode.relay
                                  ? _relayUrl.text.trim()
                                  : null,
                            ));
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
                              padding:
                                  const EdgeInsets.only(left: 16, bottom: 8),
                              child: TextField(
                                controller: _relayUrl,
                                decoration: const InputDecoration(
                                  labelText: 'relay URL (https://…)',
                                  isDense: true,
                                  border: OutlineInputBorder(),
                                ),
                                onSubmitted: (url) =>
                                    _apply(() => rust.setRelayConfig(
                                          relayMode: RelayMode.relay,
                                          customRelayUrl: url.trim(),
                                        )),
                              ),
                            ),
                          const RadioListTile<RelayMode>(
                            title: Text('no relay (local network only)'),
                            value: RelayMode.disabled,
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
    );
  }
}
