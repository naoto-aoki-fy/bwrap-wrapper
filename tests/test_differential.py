"""Opt-in subprocess comparisons against Rust main at revision 1446c5f."""

import json
import os
from pathlib import Path
import re
import signal
import tempfile
import unittest

from tests.support import ROOT, PYTHON, FakeRuntime, executable, install_wrapper
from tests.test_transform import codex


def normalize_log(contents):
    contents = re.sub(rb"(?m)^Timestamp: .*?$", b"Timestamp: <time>", contents)
    return re.sub(rb"(?m)^PID: [0-9]+$", b"PID: <pid>", contents)


@unittest.skipUnless(os.environ.get("BWRAP_RUST_BIN") and os.environ.get("BWRAP_RUST_DRIVER"),
                     "set BWRAP_RUST_BIN and BWRAP_RUST_DRIVER to compare with Rust")
class DifferentialTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(temp.cleanup)
        cls.python = (install_wrapper(Path(temp.name)),)
        cls.rust = (Path(os.environ["BWRAP_RUST_BIN"]).absolute(),)
        cls.rust_driver = (Path(os.environ["BWRAP_RUST_DRIVER"]).absolute(),)
        cls.python_driver = (PYTHON, "-ISBXutf8", ROOT / "tests/mount_driver.py")

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.runtime = FakeRuntime(self.root)
        self.runtime.env["LANG"] = "C"
        self.runtime.env["LC_CTYPE"] = ""
        self.config_root = self.runtime.settings.parent
        self.toml = self.config_root / "config.toml"
        self.mounts = self.root / "mounts"
        self.mounts.write_bytes(
            b"x /mnt/c 9p rw,aname=drvfs\n"
            b"x /mnt/c 9p aname=drvfs\n"
            b"x /mnt/c/deep 9p rw;aname=drvfs\n"
            b"x /a\\040b 9p aname=drvfs\n"
            b"x /unicode\\377 9p aname=drvfs\n"
            b"x /wrong 9p aname=drvfs-extra\n"
        )

    def settings(self, enabled, extra=()):
        self.runtime.config(json.dumps({
            "logging": {"enabled": enabled}, "codex": {"extra_read_only_dirs": extra},
        }).encode("utf-8"))
        # The shared fixture paths use the same string syntax in TOML and JSON.
        self.toml.write_text(
            f"[logging]\nenabled = {str(enabled).lower()}\n"
            f"[codex]\nextra_read_only_dirs = {json.dumps(extra, ensure_ascii=False)}\n",
            encoding="utf-8",
        )

    def compare(self, args, *, synthetic=False, env=None, status=0, failure=None, logged=False,
                log_directory=None, expected_stdout=None, input=None, project_tmp=False):
        runtime = self.runtime
        env = dict(runtime.env if env is None else env)
        if synthetic:
            env["BWRAP_TEST_MOUNTS"] = str(self.mounts)
        launchers = (self.rust_driver, self.python_driver) if synthetic else (self.rust, self.python)

        results = []
        for launcher in launchers:
            runtime.log.unlink(missing_ok=True)
            if runtime.log.parent.exists():
                runtime.log.parent.rmdir()
            result = runtime.command(launcher, args, env=env, input=input)
            self.assertEqual(result.returncode, status, (launcher, result.stderr))
            stdout = result.stdout
            if failure is None:
                stderr = result.stderr
            else:
                self.assertRegex(result.stderr, failure)
                stderr = failure
            log = None
            if logged:
                raw_log = runtime.log.read_bytes()
                self.assertTrue(raw_log.endswith(b"=========== BWRAP END ===========\n"))
                log = normalize_log(raw_log)
            if project_tmp and launcher == self.python_driver:
                option, fd, target, stdout = stdout.split(b"\0", 3)
                self.assertEqual((option, target), (b"--bind-fd", b"/tmp"))
                self.assertRegex(fd, rb"^[0-9]+$")
                if log is not None:
                    log = log.replace(b"\nModified args: --bind-fd " + fd + b" /tmp ",
                                      b"\nModified args: ", 1)
            if expected_stdout is not None:
                self.assertEqual(stdout, expected_stdout)
            if log_directory is not None:
                self.assertEqual(runtime.log.parent.exists(), log_directory)
            results.append((stdout, stderr, log))
        for result in results[1:]:
            self.assertEqual(result, results[0])

    def test_passthrough_with_complete_log_and_nonzero_child_status(self):
        self.settings(True)
        env = dict(self.runtime.env, FAKE_STREAMS="1", FAKE_EXIT="23", MULTILINE="a\nb",
                   ZZ_LAST="last", AA_FIRST="first")
        self.compare(["--dev", "/dev", "a b", "", "with'quote", "日本語"],
                     env=env, status=23, logged=True, input=b"input bytes\xff\n")

    def test_both_codex_formats_with_mount_tables_and_complete_logs(self):
        self.settings(True, ["~", "~/./ref//files/", "/absolute/../with space/"])
        folder = self.config_root / "cwd"
        folder.mkdir()
        for name in ("00-first", ".dot", "z", "é", "🐍"):
            (folder / name).touch()
        work = "/work/space"
        options = [
            "--dev", "/dev", "--bind", work + "/", work + "/",
            "--bind", work, work,
            "--ro-bind", work + "/.git", work + "/.git",
            "--perms", "000", "--tmpfs", "/tmp/.aws", "--remount-ro", "/tmp/.aws",
            "--tmpfs", "/tmp/.codex", "--remount-ro", "/tmp/.codex",
            "--perms", "000", "--tmpfs", "/tmp/.agents",
            "--bind", work, work,
        ]
        policy = ["--sandbox-policy-cwd=/work/./project/../space/"]
        command = ["--", "/bin/sh", "--sandbox-policy-cwd=relative", "a b", "", "quote's"]
        helper = str(self.runtime.home / ".codex/tmp/arg0-linux/id/codex-linux-sandbox")
        for legacy in (False, True):
            with self.subTest(legacy=legacy):
                self.compare(codex(options, policy, command, legacy=legacy, helper=helper),
                             synthetic=True, logged=True, project_tmp=True)
        self.compare(codex(), synthetic=True, logged=True)
        self.compare(["--argv0", "codex-linux-sandbox", "--bind", "source", "--", "/bin/codex"],
                     synthetic=True, logged=True)

    def test_raw_bytes_in_arguments_environment_and_overlay_names(self):
        self.settings(False)
        env = dict(self.runtime.env, RAW_NAME="a\udcfe")
        env["RAW_\udcff"] = "value"
        self.compare([b"arg\xff", b"valid\xc3\xa9", b""], env=env,
                     expected_stdout=b"arg\xff\0valid\xc3\xa9\0\0", log_directory=False)
        folder = self.config_root / "cwd"
        folder.mkdir()
        for name in ("é", "\udcff", "🐍"):
            (folder / name).touch()
        work = os.fsdecode(b"/work\xff")
        options = ["--bind", work, work, "--ro-bind", work + "/.git", work + "/.git"]
        self.compare(codex(options, ["--sandbox-policy-cwd", work], ["--", b"nested\xff"]),
                     synthetic=True, env=env, log_directory=False, project_tmp=True)

    def test_failures_and_signal_termination(self):
        self.settings(True)
        self.compare(codex(helper_options=["--sandbox-policy-cwd=relative"]),
                     synthetic=True, status=1, failure=b"must be absolute", log_directory=True)
        self.mounts.unlink()
        self.compare(codex(), synthetic=True, status=1, failure=b"No such file",
                     log_directory=True)
        self.mounts.write_bytes(b"\xff")
        self.compare(codex(), synthetic=True, status=1, failure=b"[Uu][Tt][Ff]-8",
                     log_directory=True)
        self.compare([b"raw\xff"], status=1, failure=b"failed to write invocation log",
                     log_directory=True)
        self.compare([], env=dict(self.runtime.env, PATH=""), status=1,
                     failure=b"no executable bwrap", log_directory=True)
        self.runtime.fake.write_bytes(b"invalid executable")
        self.compare([], status=1, failure=b"failed to execute", logged=True)

        self.settings(False)
        for name, status in [("PIPE", 0), ("XFSZ", -signal.SIGXFSZ), ("TERM", -signal.SIGTERM)]:
            with self.subTest(signal=name):
                executable(self.runtime.fake, f'#!/bin/sh\nkill -{name} "$$"\nprintf survived')
                self.compare([], status=status, log_directory=False)

    def test_equivalent_invalid_configuration_fails_before_logging(self):
        for obj, toml in [
            ({"logging": {"enabled": "true"}}, '[logging]\nenabled = "true"\n'),
            ({"logging": {"enabled": True}, "codex": {"extra_read_only_dirs": ["relative"]}},
             '[logging]\nenabled = true\n[codex]\nextra_read_only_dirs = ["relative"]\n'),
        ]:
            self.runtime.config(json.dumps(obj).encode("utf-8"))
            self.toml.write_text(toml, encoding="utf-8")
            self.compare([], status=1, failure=b"invalid configuration file", log_directory=False)
