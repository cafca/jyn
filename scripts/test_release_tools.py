#!/usr/bin/env python3
"""Tests for release_tools. Run: `python3 scripts/test_release_tools.py`
(zero dependencies, so no venv needed)."""
import unittest

import release_tools as rt


class PubspecVersion(unittest.TestCase):
    def test_combines_semver_and_build(self):
        self.assertEqual(rt.pubspec_version("1.2.3", 142), "1.2.3+142")

    def test_strips_whitespace_from_version_file(self):
        self.assertEqual(rt.pubspec_version("1.0.0\n", 7), "1.0.0+7")

    def test_rejects_non_semver(self):
        for bad in ["1.0", "v1.0.0", "1.0.0-rc1", "1.2.3.4"]:
            with self.assertRaises(ValueError):
                rt.pubspec_version(bad, 1)

    def test_rejects_bad_build(self):
        with self.assertRaises(ValueError):
            rt.pubspec_version("1.0.0", 0)


class SetPubspecVersion(unittest.TestCase):
    def test_replaces_top_level_version_only(self):
        src = "name: jyn\nversion: 1.0.0+1\nenvironment:\n  sdk: ^3.11.5\n"
        out = rt.set_pubspec_version(src, "1.0.1+42")
        self.assertIn("version: 1.0.1+42\n", out)
        self.assertIn("name: jyn\n", out)
        self.assertIn("  sdk: ^3.11.5\n", out)

    def test_does_not_touch_nested_version_keys(self):
        src = "version: 1.0.0+1\ndeps:\n  version: 9.9.9\n"
        out = rt.set_pubspec_version(src, "2.0.0+2")
        self.assertIn("version: 2.0.0+2\n", out)
        self.assertIn("  version: 9.9.9\n", out)

    def test_raises_when_absent(self):
        with self.assertRaises(ValueError):
            rt.set_pubspec_version("name: jyn\n", "1.0.0+1")


class RenderAppcast(unittest.TestCase):
    ITEM = {
        "short_version": "1.0.1",
        "build": 142,
        "url": "https://github.com/cafca/jyn/releases/download/v1.0.1/jyn-1.0.1.zip",
        "ed_signature": "abc+def/123==",
        "length": 4096,
        "pub_date": "Wed, 09 Jul 2026 12:00:00 +0000",
        "notes_html": "<h1>1.0.1</h1><p>Fixes.</p>",
        "minimum_system_version": "11.0",
    }

    def test_contains_required_sparkle_fields(self):
        xml = rt.render_appcast([self.ITEM])
        self.assertIn("<sparkle:version>142</sparkle:version>", xml)
        self.assertIn(
            "<sparkle:shortVersionString>1.0.1</sparkle:shortVersionString>", xml
        )
        self.assertIn('sparkle:edSignature="abc+def/123=="', xml)
        self.assertIn('length="4096"', xml)
        self.assertIn(self.ITEM["url"], xml)
        self.assertIn("<sparkle:minimumSystemVersion>11.0", xml)

    def test_release_notes_wrapped_in_cdata(self):
        xml = rt.render_appcast([self.ITEM])
        self.assertIn("<![CDATA[<h1>1.0.1</h1><p>Fixes.</p>]]>", xml)

    def test_cdata_terminator_is_escaped(self):
        item = dict(self.ITEM, notes_html="a]]>b")
        xml = rt.render_appcast([item])
        self.assertNotIn("a]]>b", xml)
        self.assertIn("]]]]><![CDATA[>", xml)

    def test_optional_fields_omitted(self):
        item = {
            "short_version": "1.0.0",
            "build": 1,
            "url": "https://example.test/jyn.zip",
            "ed_signature": "sig",
            "length": 1,
        }
        xml = rt.render_appcast([item])
        self.assertNotIn("<pubDate>", xml)
        self.assertNotIn("<![CDATA[", xml)  # no item release notes
        self.assertNotIn("minimumSystemVersion", xml)

    def test_multiple_items_render(self):
        xml = rt.render_appcast([self.ITEM, dict(self.ITEM, build=100, short_version="1.0.0")])
        self.assertEqual(xml.count("<item>"), 2)


if __name__ == "__main__":
    unittest.main()
