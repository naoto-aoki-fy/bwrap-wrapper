import errno
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from bwrap_wrapper import config, mounts


class SettingsTests(unittest.TestCase):
    def test_json_defaults_and_unknown_keys(self):
        for source in ["", " \r\t\n", "{}", '{"other":{"value":[null,42]}}',
                       '{"other":' + "9" * 5000 + '}',
                       '{"codex":{},"logging":{"enabled":false,"other":true}}']:
            with self.subTest(source=source):
                self.assertEqual(config.parse(source, "/home/u"), config.Settings())

    def test_path_expansion_retains_rust_spelling(self):
        source = """{"logging":{"enabled":true},"codex":{"extra_read_only_dirs":[
            "~","~/","~/./shared//a/","~//.././a/","/already//./absolute/",
            "//double-root/","/unicode/🐍"
        ]}}"""
        self.assertEqual(config.parse(source, "/home/u"), config.Settings(True, (
            "/home/u/", "/home/u/", "/home/u/shared//a", "/home/u/.././a",
            "/already//./absolute/", "//double-root/", "/unicode/🐍",
        )))

    def test_invalid_json_and_supported_settings(self):
        for source in [
            "null", "[]", "true", "{", "{}{}", '{"logging":{},}', '{"logging":null}',
            '{"logging":[]}', '{"codex":false}', '{"logging":{"enabled":"true"}}',
            '{"logging":{"enabled":1}}', '{"codex":{"extra_read_only_dirs":"~/dir"}}',
            '{"codex":{"extra_read_only_dirs":[false]}}',
            '{"codex":{"extra_read_only_dirs":["relative"]}}',
            '{"codex":{"extra_read_only_dirs":["~other/dir"]}}',
            r'{"codex":{"extra_read_only_dirs":["/bad\u0000path"]}}',
            r'{"codex":{"extra_read_only_dirs":["/bad\udcffpath"]}}',
            '{"logging":{},"logging":{}}', '{"other":{"same":1,"same":2}}',
            '{"other":NaN}', '{"other":Infinity}', '{"other":-Infinity}',
        ]:
            with self.subTest(source=source):
                with self.assertRaises(ValueError):
                    config.parse(source, "/home/u")
        with self.assertRaisesRegex(ValueError, "absolute"):
            config.parse('{"codex":{"extra_read_only_dirs":["~"]}}', "relative")

    def test_config_location_and_read_errors(self):
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp)
            default = home / ".config/bwrap-wrapper/config.json"
            self.assertEqual(config.load(temp, None), config.Settings())
            default.parent.mkdir(parents=True)
            default.write_text('{"logging":{"enabled":true}}', encoding="utf-8")
            self.assertTrue(config.load(temp, "").logging_enabled)
            self.assertEqual(config.load(temp, str(home / "xdg")), config.Settings())

            default.write_bytes(b"\xff")
            with self.assertRaisesRegex(ValueError, "invalid configuration file.*config.json"):
                config.load(temp, None)
            default.unlink()
            default.mkdir()
            with self.assertRaisesRegex(ValueError, "failed to read configuration file"):
                config.load(temp, None)

    def test_relative_config_root_preserves_parent_components(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "link").symlink_to(root / "real/deep")
            (root / "real/deep").mkdir(parents=True)
            folder = root / "real/xdg/bwrap-wrapper/cwd"
            folder.mkdir(parents=True)
            (folder / "overlay").touch()
            (folder.parent / "config.json").write_text('{"logging":{"enabled":true}}',
                                                        encoding="utf-8")
            with patch("os.getcwd", return_value=temp):
                self.assertTrue(config.load("/unused", "link/../xdg/./").logging_enabled)
                self.assertEqual(config.discover("/unused", "link/../xdg/./"),
                                 [str(root / "link/../xdg/bwrap-wrapper/cwd/overlay")])


class OverlayTests(unittest.TestCase):
    def test_direct_files_sorted_with_surrogateescape(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "bwrap-wrapper/cwd"
            folder.mkdir(parents=True)
            for name in ["z", "🐍", os.fsdecode(b"\xff"), ".dot", "é"]:
                (folder / name).touch()
            (folder / "dir").mkdir()
            (folder / "dir/hidden").touch()
            (folder / "link").symlink_to("z")
            (folder / "link-dir").symlink_to("dir")
            (folder / "broken").symlink_to("missing")
            (folder / "loop").symlink_to("loop")
            os.mkfifo(folder / "fifo")
            self.assertEqual(
                [Path(path).name for path in config.discover("/unused", temp)],
                [".dot", "link", "z", "é", os.fsdecode(b"\xff"), "🐍"],
            )

    def test_missing_or_non_directory_and_non_missing_errors(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "bwrap-wrapper/cwd"
            folder.parent.mkdir()
            self.assertEqual(config.discover("/unused", temp), [])
            folder.touch()
            self.assertEqual(config.discover("/unused", temp), [])
            folder.unlink()
            folder.mkdir()
            (folder / "bad").symlink_to("x" * 300)
            with self.assertRaises(OSError) as cm:
                config.discover("/unused", temp)
            self.assertEqual(cm.exception.errno, errno.ENAMETOOLONG)
            with patch.object(Path, "iterdir", side_effect=PermissionError(errno.EACCES, "denied")):
                with self.assertRaises(PermissionError):
                    config.discover("/unused", temp)


class MountTests(unittest.TestCase):
    def test_exact_options_escapes_deduplication_and_order(self):
        table = (
            "too short\n"
            "x /deep/a/b 9p rw;aname=drvfs;foo\r"
            "x /deep/a 9p rw,aname=drvfs\r\n"
            "x /deep\\057a 9p aname=drvfs\n"
            "x\x1c/z\x1f9p\x1daname=drvfs\n"
            "x /mnt/a\\040space 9p aname=drvfs\n"
            "x /mnt/b\\134path 9p aname=drvfs\n"
            "x /literal\\08x 9p aname=drvfs\n"
            "x /unicode\\377 9p aname=drvfs\n"
            "x /wrong ext4 aname=drvfs\nx /wrong 9P aname=drvfs\n"
            "x /wrong 9p prefixaname=drvfs\nx /wrong 9p aname=drvfs-extra\n"
            "x /wrong 9p other aname=drvfs\n"
        )
        self.assertEqual(mounts.parse(table), [
            "/literal\\08x", "/unicodeÿ", "/z", "/deep/a", "/mnt/a space",
            "/mnt/b\\path", "/deep/a/b",
        ])

    def test_mount_read_errors_and_strict_utf8(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "mounts"
            with self.assertRaises(FileNotFoundError):
                mounts.read_file(path)
            path.write_bytes(b"\xff")
            with self.assertRaises(UnicodeDecodeError):
                mounts.read_file(path)
