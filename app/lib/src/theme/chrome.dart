/// Shared window chrome for the content-first direction: the titlebar
/// strip under the (transparent) native titlebar, the 56px toolbar, the
/// wordmark, and the "upcoming" affordance for not-yet-backed features.
library;

import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;

import 'tokens.dart';

/// The 34px strip the macOS traffic lights float over. The native titlebar
/// is transparent + full-size-content (see MainFlutterWindow.swift), so
/// this just reserves the height — body-colored, borderless, blending
/// seamlessly into the window. Skipped on platforms that keep their own
/// titlebar.
class JynTitlebarStrip extends StatelessWidget {
  const JynTitlebarStrip({super.key});

  @override
  Widget build(BuildContext context) {
    if (!Platform.isMacOS) return const SizedBox.shrink();
    return Container(height: JynLayout.titlebarInset, color: JynColors.body);
  }
}

/// Constrains one scroll-list item to the 440px column while the list
/// itself spans the window — so the scrollbar rides the window edge.
class JynColumnItem extends StatelessWidget {
  const JynColumnItem({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: JynLayout.column),
        child: child,
      ),
    );
  }
}

/// The 56px toolbar: optional back chevron, the wordmark (always a way
/// home), an optional title, then actions pushed right.
class JynToolbar extends StatelessWidget {
  const JynToolbar({
    super.key,
    this.showBack = false,
    this.title,
    this.onWordmarkTap,
    this.actions = const [],
  });

  final bool showBack;
  final String? title;

  /// Home overrides this with scroll-to-top; elsewhere the default pops
  /// to the root (the wordmark is a global "go home").
  final VoidCallback? onWordmarkTap;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: JynLayout.toolbarHeight,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      decoration: const BoxDecoration(
        color: JynColors.body,
        border: Border(bottom: BorderSide(color: JynColors.hairlineFaint)),
      ),
      child: Row(
        children: [
          // The wordmark holds the same leftmost spot on every screen;
          // the back chevron slots in after it.
          JynWordmark(
            onTap:
                onWordmarkTap ??
                () => Navigator.of(context).popUntil((r) => r.isFirst),
          ),
          if (showBack) ...[
            const SizedBox(width: 10),
            _HoverIcon(
              icon: Icons.chevron_left,
              tooltip: 'back',
              onTap: () => Navigator.of(context).maybePop(),
            ),
          ],
          if (title != null) ...[
            SizedBox(width: showBack ? 4 : 14),
            Text(
              title!,
              style: JynType.name.copyWith(color: JynColors.textSoft),
            ),
          ],
          const Spacer(),
          ...actions,
        ],
      ),
    );
  }
}

/// Leaf + `jyn`, the brand lockup. Tapping always leads home.
class JynWordmark extends StatelessWidget {
  const JynWordmark({super.key, this.onTap});

  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: const [
            Icon(Icons.eco, size: 18, color: JynColors.leaf),
            SizedBox(width: 5),
            Text('jyn', style: JynType.wordmark),
          ],
        ),
      ),
    );
  }
}

/// The toolbar search field. Search has no core backing yet, so it renders
/// per the design but inert, with the upcoming tooltip.
class JynSearchField extends StatelessWidget {
  const JynSearchField({super.key, this.width = 200});

  final double width;

  @override
  Widget build(BuildContext context) {
    return Upcoming(
      message: 'search is coming soon',
      child: Container(
        width: width,
        height: 32,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        decoration: BoxDecoration(
          color: JynColors.field,
          borderRadius: BorderRadius.circular(9),
        ),
        child: Row(
          children: [
            const Icon(Icons.search, size: 16, color: JynColors.secondary),
            const SizedBox(width: 6),
            Text(
              'search the river',
              style: JynType.body.copyWith(
                fontSize: 13,
                color: JynColors.muted,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Wraps an affordance whose feature the core doesn't support yet:
/// visually present (slightly muted), inert, with a hover note.
class Upcoming extends StatelessWidget {
  const Upcoming({super.key, required this.message, required this.child});

  final String message;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: message,
      child: MouseRegion(
        cursor: SystemMouseCursors.basic,
        child: Opacity(opacity: 0.55, child: IgnorePointer(child: child)),
      ),
    );
  }
}

/// A hoverable icon affordance without Material ink.
class _HoverIcon extends StatelessWidget {
  const _HoverIcon({required this.icon, required this.onTap, this.tooltip});

  final IconData icon;
  final VoidCallback onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final button = MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(4),
          child: Icon(icon, size: 22, color: JynColors.textSoft),
        ),
      ),
    );
    return tooltip == null ? button : Tooltip(message: tooltip!, child: button);
  }
}

/// Toolbar icon affordance (public flavor of [_HoverIcon]).
class JynToolbarIcon extends StatelessWidget {
  const JynToolbarIcon({
    super.key,
    required this.icon,
    required this.onTap,
    this.tooltip,
  });

  final IconData icon;
  final VoidCallback onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    return _HoverIcon(icon: icon, onTap: onTap, tooltip: tooltip);
  }
}

/// A 1px hairline separator (posts, sections).
class JynHairline extends StatelessWidget {
  const JynHairline({super.key, this.faint = false});

  final bool faint;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 1,
      color: faint ? JynColors.hairlineFaint : JynColors.hairline,
    );
  }
}
