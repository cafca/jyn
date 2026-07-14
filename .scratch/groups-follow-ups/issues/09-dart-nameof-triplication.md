# 09 — The group-name resolver is copied in three Dart screens

**Status:** needs-triage

**Context:** review cleanup deferred from PR #9. `app/lib/src/screens/`
(`groups_hub_screen.dart`, `group_place_screen.dart`,
`group_admin_screen.dart`).

## Problem

Each screen carries its own `nameOf`-style helper resolving a group id to a
display name with the same fallback chain. Three copies drift the moment the
fallback changes (e.g. renamed-group handling).

## Fix direction

One helper in a shared location (e.g. `app/lib/src/actions.dart` or a
`group_display.dart`), used by all three.
