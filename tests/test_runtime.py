import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import unittest

from tests.support import FakeRuntime, executable, install_wrapper
from tests.test_transform import codex


class RuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(temp.cleanup)
        cls.launcher = install_wrapper(Path(temp.name) / "tool with spaces")
        cls.other_launcher = install_wrapper(Path(temp.name) / "other")

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.runtime = FakeRuntime(self.root)

    def test_aliases_and_other_installations_are_skipped(self):
        symlink = self.root / "symlink/bwrap"
        hardlink = self.root / "hardlink/bwrap"
        legacy = self.root / "legacy/bwrap"
        for path in (symlink, hardlink, legacy):
            path.parent.mkdir()
        symlink.symlink_to(self.launcher)
        os.link(self.launcher.resolve(), hardlink)
        executable(legacy, "#!/bin/sh\n# bwrap-wrapper Python launcher\nexit 99\n")
        self.assertFalse(os.path.samefile(self.launcher, self.other_launcher))

        env = dict(self.runtime.env, PATH=os.pathsep.join(map(str, (
            symlink.parent, hardlink.parent, self.other_launcher.parent, legacy.parent,
            self.launcher.parent, self.runtime.real,
        ))))
        for command in ("bwrap", self.launcher, self.launcher.resolve()):
            with self.subTest(command=command):
                result = self.runtime.command([command], ["--version"], env=env)
                self.assertEqual((result.returncode, result.stdout, result.stderr),
                                 (0, b"--version\0", b""))
        self.assertFalse((self.runtime.home / ".cache").exists())

    def test_installed_launcher_applies_codex_rewrite(self):
        workspace = "/workspace"
        protected = "/workspace/.git"
        policy = "--sandbox-policy-cwd=/a/../workspace/"
        args = [
            "--ro-bind", "/", "/", "--bind", "/tmp", "/tmp",
            "--dev", "/dev", "--ro-bind", protected, protected,
            "--bind", workspace, workspace,
            # Keep the fixture independent of host mounts by consuming the separator.
            "--bind", "source", "--", "/bin/codex-linux-sandbox", policy,
        ]
        home = str(self.runtime.home)
        overlay = self.runtime.settings.parent / "cwd/example"
        overlay.parent.mkdir(parents=True)
        overlay.touch()
        packages = home + "/.codex/packages"
        memories = home + "/.codex/memories"
        agents = home + "/.codex/AGENTS.md"
        cache = self.runtime.home / ".cache/codex/tmp/workspace"
        marker = cache / "kept"
        for attempt in range(2):
            result = self.runtime.command([self.launcher], args, umask=0)
            self.assertEqual((result.returncode, result.stderr), (0, b""))
            forwarded = os.fsdecode(result.stdout).split("\0")[:-1]
            self.assertRegex(forwarded[4], r"^\d+$")
            self.assertEqual(forwarded, [
                "--ro-bind", "/", "/", "--bind-fd", forwarded[4], "/tmp",
                "--dev-bind", "/dev", "/dev",
                "--tmpfs", home, "--dir", home + "/.codex",
                "--ro-bind-try", packages, packages, "--bind-try", memories, memories,
                "--ro-bind-try", agents, agents,
                "--bind", workspace, workspace, "--ro-bind", str(overlay), "/workspace/example",
                "--bind", "source", "--", "/bin/codex-linux-sandbox", policy,
            ])
            self.assertEqual(cache.stat().st_mode & 0o777, 0o700)
            if attempt == 0:
                marker.write_bytes(b"kept")
            else:
                self.assertEqual(marker.read_bytes(), b"kept")

    def test_project_tmp_rejects_symlinked_project_components(self):
        target = self.root / "other"
        target.mkdir()
        cache = self.runtime.home / ".cache/codex/tmp"
        cache.mkdir(parents=True)
        (cache / "projects").symlink_to(target)
        args = codex(helper_options=["--sandbox-policy-cwd=/projects/workspace"])
        result = self.runtime.command([self.launcher], args)
        self.assertEqual((result.returncode, result.stdout), (1, b""))
        self.assertIn(b"failed to prepare project /tmp", result.stderr)
        self.assertEqual(list(target.iterdir()), [])

    def test_path_search_order_includes_empty_and_relative_entries(self):
        skipped = self.root / "skip"
        skipped.mkdir()
        (skipped / "bwrap").mkdir()
        ignored = self.root / "ignored"
        ignored.mkdir()
        (ignored / "bwrap").write_text("not executable", encoding="utf-8")
        broken = self.root / "broken"
        broken.mkdir()
        (broken / "bwrap").symlink_to("missing")
        local = executable(self.root / "bwrap", "#!/bin/sh\nprintf local")
        env = dict(self.runtime.env, PATH="skip:ignored:broken::real")
        first = self.runtime.command([self.launcher], ["x"], env=env)
        self.assertEqual((first.returncode, first.stdout, first.stderr), (0, b"local", b""))
        local.unlink()
        later = self.runtime.command([self.launcher], ["x"], env=env)
        self.assertEqual((later.returncode, later.stdout, later.stderr), (0, b"x\0", b""))
        env.pop("PATH")
        missing = self.runtime.command([self.launcher], env=env)
        self.assertIn(b"PATH is not set", missing.stderr)
        env["PATH"] = os.pathsep.join(map(str, (self.launcher.parent, self.other_launcher.parent)))
        missing = self.runtime.command([self.launcher], env=env)
        self.assertEqual(missing.returncode, 1)
        self.assertIn(b"no executable bwrap", missing.stderr)

    def test_launcher_isolates_imports_and_forwards_pid_streams_environment_and_status(self):
        runtime = self.runtime
        poison = self.root / "poison"
        poison.mkdir()
        (poison / "sitecustomize.py").write_text("raise SystemExit(42)", encoding="utf-8")
        (poison / "bwrap_wrapper.py").write_text("raise SystemExit(43)", encoding="utf-8")
        pid_file = self.root / "pid"
        env_file = self.root / "env"
        env = dict(runtime.env, PYTHONPATH=str(poison), PYTHONHOME=str(poison), PYTHONUTF8="0",
                   PYTHONCOERCECLOCALE="warn", LANG="C", FAKE_PID_FILE=str(pid_file),
                   FAKE_ENV=str(env_file), FAKE_EXIT="23", FAKE_STREAMS="1")
        for lc_ctype in (None, "C", "POSIX", "", "C.UTF-8", "en_US.UTF-8"):
            with self.subTest(lc_ctype=lc_ctype):
                child_env = dict(env)
                if lc_ctype is not None:
                    child_env["LC_CTYPE"] = lc_ctype
                with subprocess.Popen(
                    [self.launcher, "two words", "", "a'b", "é"], env=child_env, cwd=poison,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                ) as process:
                    stdout, stderr = process.communicate(b"from stdin\n", timeout=15)
                    self.assertEqual(process.returncode, 23, stderr)
                    self.assertEqual(pid_file.read_text(encoding="ascii"), str(process.pid))
                self.assertEqual(stdout, b"two words\0\0a'b\0\xc3\xa9\0from stdin\n")
                self.assertEqual(stderr, b"fake stderr\n")
                self.assertEqual(set(env_file.read_bytes().split(b"\0")[:-1]), {
                    os.fsencode(f"{key}={value}") for key, value in child_env.items()
                })

    def test_log_off_preserves_raw_arguments_environment_and_existing_log(self):
        runtime = self.runtime
        env_file = self.root / "env"
        env = {os.fsencode(key): os.fsencode(value) for key, value in runtime.env.items()}
        env[b"FAKE_ENV"] = os.fsencode(env_file)
        env[b"NAME_\xff"] = b"value\xfe"
        env[b"LC_CTYPE"] = b"\xfe"
        result = runtime.command([self.launcher], [b"/path\xff", b"\xfe", b""], env=env)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"/path\xff\0\xfe\0\0", b""))
        self.assertIn(b"NAME_\xff=value\xfe\0", env_file.read_bytes())
        self.assertIn(b"LC_CTYPE=\xfe\0", env_file.read_bytes())
        self.assertFalse(runtime.log.parent.exists())
        runtime.log.parent.mkdir()
        runtime.log.write_bytes(b"existing\xff")
        runtime.config(b'{"logging":{"enabled":false}}')
        for path in (str(runtime.log), "", str(runtime.log.parent)):
            env = dict(runtime.env, BWRAP_LOG=path)
            result = runtime.command([self.launcher], ["x"], env=env)
            self.assertEqual((result.returncode, result.stderr), (0, b""))
        self.assertEqual(runtime.log.read_bytes(), b"existing\xff")

    def test_log_permissions_shell_quoting_and_sorted_environment(self):
        runtime = self.runtime
        runtime.enable_logging()
        env = dict(runtime.env, ZZ_LAST="last", AA_FIRST="first", LC_ALL="C")
        args = ["a b", "", "quote's", "é", "--dev", "/dev"]
        for _ in range(2):
            result = runtime.command([self.launcher], args, env=env, umask=0)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(runtime.log.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual(runtime.log.stat().st_mode & 0o777, 0o600)
        log = runtime.log.read_text(encoding="utf-8")
        quoted = """'a b' '' 'quote'"'"'s' 'é' --dev /dev"""
        self.assertEqual(log.count("========== BWRAP START ==========\n"), 2)
        self.assertEqual(log.count("=========== BWRAP END ===========\n"), 2)
        self.assertEqual(log.count(f"Original args: {quoted}\nModified args: {quoted}\n"), 2)
        self.assertIn("Codex invocation: False\n", log)
        self.assertIn(f"Bwrap: {runtime.fake}\nSandbox CWD: (not specified)\n", log)
        self.assertRegex(log, re.compile(r"^Timestamp: .+\nUser: .+\nPID: [0-9]+$", re.M))
        sorted_env = "\n".join(f"{key}={value}" for key, value in sorted(env.items()))
        self.assertEqual(log.count(f"Environment:\n{sorted_env}\n"), 2)
        runtime.log.chmod(0o640)
        env.pop("BWRAP_LOG")
        fallback = runtime.command([self.launcher], [], env=env)
        self.assertEqual(fallback.returncode, 0, fallback.stderr)
        self.assertEqual((runtime.home / ".local/state/bwrap-wrapper.log").stat().st_mode & 0o777,
                         0o600)
        self.assertEqual(runtime.log.stat().st_mode & 0o777, 0o640)

    def test_startup_and_log_failure_order(self):
        runtime = self.runtime
        env = dict(runtime.env)
        env.pop("HOME")
        missing_home = runtime.command([self.launcher], env=env)
        self.assertEqual(missing_home.returncode, 1)
        self.assertIn(b"HOME environment variable is required", missing_home.stderr)
        for bad in (b"{", b"\xff", b'{"logging":{"enabled":true},"codex":[]}',
                    b'{"codex":{"extra_read_only_dirs":["relative"]}}'):
            runtime.config(bad)
            result = runtime.command([self.launcher], ["non-Codex"])
            self.assertEqual((result.returncode, result.stdout), (1, b""))
            self.assertIn(b"invalid configuration file", result.stderr)
            self.assertFalse(runtime.log.parent.exists())

        runtime.enable_logging()
        relative = codex(helper_options=["--sandbox-policy-cwd=relative"])
        env = dict(runtime.env, BWRAP_LOG=str(runtime.settings / "blocked/wrapper.log"))
        result = runtime.command([self.launcher], relative, env=env)
        self.assertIn(b"failed to prepare invocation log", result.stderr)
        self.assertFalse(runtime.log.parent.exists())

        result = runtime.command([self.launcher], relative)
        self.assertIn(b"must be absolute", result.stderr)
        self.assertTrue(runtime.log.parent.is_dir())
        self.assertFalse(runtime.log.exists())

        env = dict(runtime.env, PATH=str(self.root / "no-real"))
        result = runtime.command([self.launcher], [b"arg\xff"], env=env)
        self.assertIn(b"no executable bwrap", result.stderr)
        self.assertFalse(runtime.log.exists())

        for args, env in [([b"arg\xff"], runtime.env), ([], dict(runtime.env, RAW="\udcff"))]:
            result = runtime.command([self.launcher], args, env=env)
            self.assertEqual((result.returncode, result.stdout), (1, b""))
            self.assertIn(b"failed to write invocation log", result.stderr)
            self.assertNotIn(b"BWRAP END", runtime.log.read_bytes())
        result = runtime.command([self.launcher], env=dict(runtime.env, BWRAP_LOG=""))
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"failed to write invocation log", result.stderr)

    def test_log_written_before_exec_failure(self):
        runtime = self.runtime
        runtime.enable_logging()
        runtime.fake.write_bytes(b"not a valid executable")
        result = runtime.command([self.launcher], ["argument"])
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"failed to execute", result.stderr)
        self.assertTrue(runtime.log.read_bytes().endswith(b"=========== BWRAP END ===========\n"))

    def test_signal_dispositions_and_termination(self):
        for name, status in [("PIPE", 0), ("XFSZ", -signal.SIGXFSZ), ("TERM", -signal.SIGTERM)]:
            with self.subTest(signal=name):
                executable(self.runtime.fake, f'#!/bin/sh\nkill -{name} "$$"\nprintf survived')
                result = self.runtime.command([self.launcher])
                self.assertEqual(result.returncode, status, result.stderr)
                self.assertEqual(result.stdout, b"survived" if status == 0 else b"")
