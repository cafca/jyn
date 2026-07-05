import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../providers.dart';
import '../rust/diagnostics.dart';

/// Node internals: identity, peers, gossip topics, history, errors.
/// Snapshots refresh once a second while the runtime is up.
class DiagnosticsScreen extends ConsumerWidget {
  const DiagnosticsScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final snapshot = ref.watch(diagnosticsProvider);

    return Scaffold(
      appBar: AppBar(title: const Text('diagnostics')),
      body: snapshot == null
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.all(16),
              children: [
                _section(theme, 'node'),
                _mono(theme, snapshot.nodeIdentity.nodeId),
                if (snapshot.nodeIdentity.relayUrl != null)
                  _mono(theme, 'relay: ${snapshot.nodeIdentity.relayUrl}'),
                for (final addr in snapshot.nodeIdentity.localListenAddrs)
                  _mono(theme, addr),
                _section(theme, 'peers (${snapshot.peers.length})'),
                if (snapshot.peers.isEmpty)
                  Text('nobody in sight', style: theme.textTheme.bodySmall),
                for (final peer in snapshot.peers) _peerTile(theme, peer),
                _section(
                  theme,
                  'gossip topics (${snapshot.gossipTopics.length})',
                ),
                for (final topic in snapshot.gossipTopics)
                  _mono(
                    theme,
                    '${topic.topicId}  ·  ${topic.peerCount} peer(s)',
                  ),
                _section(theme, 'connection history'),
                for (final entry in snapshot.connectionHistory.reversed.take(
                  30,
                ))
                  _mono(
                    theme,
                    '${_time(entry.atUnixMs)}  ${entry.event}'
                    '${entry.peerNodeId != null ? '  ${_short(entry.peerNodeId!)}' : ''}'
                    '${entry.detail.isNotEmpty ? '  — ${entry.detail}' : ''}',
                  ),
                _section(theme, 'errors'),
                if (snapshot.errorLog.isEmpty)
                  Text('none', style: theme.textTheme.bodySmall),
                for (final error in snapshot.errorLog.reversed.take(20))
                  _mono(theme, '${_time(error.atUnixMs)}  ${error.message}'),
              ],
            ),
    );
  }

  Widget _section(ThemeData theme, String title) => Padding(
    padding: const EdgeInsets.only(top: 16, bottom: 8),
    child: Text(title, style: theme.textTheme.titleSmall),
  );

  Widget _mono(ThemeData theme, String text) => Padding(
    padding: const EdgeInsets.only(bottom: 2),
    child: SelectableText(
      text,
      style: theme.textTheme.bodySmall?.copyWith(fontFamily: 'monospace'),
    ),
  );

  Widget _peerTile(ThemeData theme, PeerSnapshot peer) {
    final state = switch (peer.state) {
      PeerConnectionState.connected => '● connected',
      PeerConnectionState.known => '◐ known',
      PeerConnectionState.disconnected => '○ disconnected',
    };
    final via = switch (peer.discoveredVia) {
      PeerDiscoveryMethod.manual => 'manual',
      PeerDiscoveryMethod.relay => 'relay',
      PeerDiscoveryMethod.mdns => 'mdns',
      PeerDiscoveryMethod.unknown => '?',
    };
    final rtt = peer.rttMs != null ? '  ${peer.rttMs}ms' : '';
    return _mono(theme, '${_short(peer.nodeId)}  $state  via $via$rtt');
  }

  String _short(String id) => id.length <= 12 ? id : id.substring(0, 12);

  String _time(int unixMs) {
    final time = DateTime.fromMillisecondsSinceEpoch(unixMs);
    String pad(int n) => n.toString().padLeft(2, '0');
    return '${pad(time.hour)}:${pad(time.minute)}:${pad(time.second)}';
  }
}
