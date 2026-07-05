#!/usr/bin/env python3
"""Pure helpers for the jyn release pipeline: project the VERSION file into
Flutter's pubspec version and render the Sparkle appcast.

Kept dependency-free (stdlib only) and side-effect-light so the interesting
logic — version projection and XML rendering — is unit-testable without a
signed build. See scripts/test_release_tools.py and scripts/release.sh.
"""
from __future__ import annotations

import json
import re
import sys
from xml.sax.saxutils import escape, quoteattr

FEED_TITLE = "jyn"
FEED_LINK = "https://cafca.github.io/jyn/appcast.xml"
FEED_DESCRIPTION = "Updates for jyn."


def pubspec_version(semver: str, build: int) -> str:
    """Flutter version string: `<semver>+<build>`.

    `semver` becomes CFBundleShortVersionString (human-facing); `build`, a
    monotonic integer (git commit count), becomes CFBundleVersion, which is
    what Sparkle compares to decide "is this newer?".
    """
    semver = semver.strip()
    if not re.fullmatch(r"\d+\.\d+\.\d+", semver):
        raise ValueError(f"VERSION must be x.y.z, got {semver!r}")
    if build < 1:
        raise ValueError(f"build number must be >= 1, got {build}")
    return f"{semver}+{build}"


def set_pubspec_version(pubspec_text: str, version: str) -> str:
    """Return `pubspec_text` with its top-level `version:` line set to `version`.

    Matches only a line-start `version:` key so nested keys are untouched.
    """
    new, n = re.subn(
        r"(?m)^version:.*$", f"version: {version}", pubspec_text, count=1
    )
    if n != 1:
        raise ValueError("could not find a top-level `version:` line in pubspec")
    return new


def _cdata(html: str) -> str:
    # A CDATA section can't contain the literal "]]>"; split it if present.
    return "<![CDATA[" + html.replace("]]>", "]]]]><![CDATA[>") + "]]>"


def render_item(item: dict) -> str:
    """Render one <item>. Required keys: short_version, build, url,
    ed_signature, length. Optional: pub_date, notes_html, minimum_system_version.
    """
    lines = ["    <item>"]
    lines.append(f"      <title>{escape(str(item['short_version']))}</title>")
    if item.get("pub_date"):
        lines.append(f"      <pubDate>{escape(item['pub_date'])}</pubDate>")
    lines.append(
        f"      <sparkle:version>{int(item['build'])}</sparkle:version>"
    )
    lines.append(
        "      <sparkle:shortVersionString>"
        f"{escape(str(item['short_version']))}</sparkle:shortVersionString>"
    )
    if item.get("minimum_system_version"):
        lines.append(
            "      <sparkle:minimumSystemVersion>"
            f"{escape(item['minimum_system_version'])}"
            "</sparkle:minimumSystemVersion>"
        )
    if item.get("notes_html"):
        lines.append(f"      <description>{_cdata(item['notes_html'])}</description>")
    lines.append(
        "      <enclosure "
        f"url={quoteattr(item['url'])} "
        f'sparkle:edSignature={quoteattr(item["ed_signature"])} '
        f'length={quoteattr(str(item["length"]))} '
        'type="application/octet-stream" />'
    )
    lines.append("    </item>")
    return "\n".join(lines)


def render_appcast(items: list[dict]) -> str:
    """Render a full appcast for the given items (newest first)."""
    body = "\n".join(render_item(i) for i in items)
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<rss version="2.0" '
        'xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/">\n'
        "  <channel>\n"
        f"    <title>{escape(FEED_TITLE)}</title>\n"
        f"    <link>{escape(FEED_LINK)}</link>\n"
        f"    <description>{escape(FEED_DESCRIPTION)}</description>\n"
        "    <language>en</language>\n"
        f"{body}\n"
        "  </channel>\n"
        "</rss>\n"
    )


def _main(argv: list[str]) -> int:
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    cmd, rest = argv[0], argv[1:]
    if cmd == "pubspec-version":
        semver, build = rest
        print(pubspec_version(semver, int(build)))
    elif cmd == "sync-pubspec":
        semver, build, path = rest
        with open(path, "r", encoding="utf-8") as f:
            text = f.read()
        text = set_pubspec_version(text, pubspec_version(semver, int(build)))
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"pubspec version -> {pubspec_version(semver, int(build))}")
    elif cmd == "appcast":
        # Reads a JSON item (or list of items) from stdin, prints the appcast.
        payload = json.load(sys.stdin)
        items = payload if isinstance(payload, list) else [payload]
        sys.stdout.write(render_appcast(items))
    else:
        print(f"unknown command: {cmd}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(_main(sys.argv[1:]))
