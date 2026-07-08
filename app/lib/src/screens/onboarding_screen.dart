import 'package:flutter/material.dart' hide Visibility;

import '../actions.dart';
import '../rust/api/commands.dart';
import '../rust/profile.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/jyn_avatar.dart';

/// The first-hour flow: pick a name, learn the one rule (lifetime is the
/// only thing separating posts), step into the river. Same single step as
/// ever, dressed in the content-first direction.
class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key, required this.profile});

  final UserProfile profile;

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  late final TextEditingController _name = TextEditingController(
    text: widget.profile.displayName,
  );
  bool _busy = false;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _enter() async {
    if (_name.text.trim().isEmpty) return;
    setState(() => _busy = true);
    await runGuarded(context, () async {
      await updateProfile(
        displayName: _name.text.trim(),
        bio: widget.profile.bio,
        defaultVisibility: widget.profile.defaultVisibility,
        defaultLifetimeSecs: widget.profile.defaultLifetimeSecs,
        markOnboarded: true,
      );
    });
    if (mounted) setState(() => _busy = false);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          Expanded(
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 420),
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: const [
                          Icon(Icons.eco, size: 26, color: JynColors.leaf),
                          SizedBox(width: 8),
                          Text(
                            'jyn',
                            style: TextStyle(
                              fontSize: 34,
                              fontWeight: FontWeight.w700,
                              color: JynColors.ink,
                              letterSpacing: -1.0,
                              height: 1.0,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 14),
                      Text(
                        'One river of posts, shared with friends you choose. '
                        'Ephemeral posts drain away; settled ones stay. '
                        'What should your friends call you?',
                        style: JynType.body.copyWith(
                          fontSize: 15,
                          color: JynColors.textSoft,
                        ),
                      ),
                      const SizedBox(height: 28),
                      Row(
                        children: [
                          JynAvatar(
                            profileId: widget.profile.profileId,
                            displayName: _name.text,
                            size: 44,
                            isSelf: true,
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: TextField(
                              controller: _name,
                              autofocus: true,
                              style: JynType.body.copyWith(fontSize: 15),
                              cursorColor: JynColors.leaf,
                              cursorWidth: 2,
                              decoration: InputDecoration(
                                hintText: 'your name',
                                hintStyle: JynType.body.copyWith(
                                  fontSize: 15,
                                  color: JynColors.muted,
                                ),
                                filled: true,
                                fillColor: JynColors.field,
                                isDense: true,
                                contentPadding: const EdgeInsets.symmetric(
                                  horizontal: 12,
                                  vertical: 11,
                                ),
                                border: OutlineInputBorder(
                                  borderRadius: BorderRadius.circular(
                                    JynRadii.attach,
                                  ),
                                  borderSide: BorderSide.none,
                                ),
                              ),
                              onChanged: (_) => setState(() {}),
                              onSubmitted: (_) => _enter(),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 20),
                      _enterButton(),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _enterButton() {
    final enabled = !_busy && _name.text.trim().isNotEmpty;
    return MouseRegion(
      cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
      child: GestureDetector(
        onTap: enabled ? _enter : null,
        child: AnimatedOpacity(
          duration: const Duration(milliseconds: 120),
          opacity: enabled ? 1 : 0.45,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 11),
            decoration: BoxDecoration(
              color: JynColors.leaf,
              borderRadius: BorderRadius.circular(JynRadii.button),
              boxShadow: enabled ? JynShadows.primaryButton : null,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: const [
                Icon(Icons.eco, size: 16, color: Colors.white),
                SizedBox(width: 8),
                Text(
                  'step into the river',
                  style: TextStyle(
                    fontSize: 14,
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
}
