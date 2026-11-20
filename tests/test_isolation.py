"""Opt-in mount namespace tests using real Bubblewrap and synthetic directories."""

import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest

from tests.support import executable, install_both

HELPER = """#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = -- ]; then shift; exec "$@"; fi
    shift
done
exit 99
"""

PROBE = r"""
deny_write() {
    if (printf bad > "$1") 2>/dev/null; then
        echo "unexpected write to $1" >&2
        exit 1
    fi
}
[ ! -e "$HOME/private" ]
[ ! -e "$HOME/.codex/tmp/unrelated" ]
[ ! -e "$HOME/.codex/tmp/arg0-linux/other/hidden" ]
[ ! -e "$HOME/.codex/shell_snapshots/snapshot.sh" ]
[ "$(stat -f -c %T "$HOME")" = tmpfs ]
if [ "$GENERATED" != "" ]; then
    [ -L "$GENERATED/codex-linux-sandbox" ]
    deny_write "$GENERATED/new"
fi
if [ "$PACKAGES" = 1 ]; then
    [ "$(cat "$HOME/.codex/packages/data")" = package ]
    deny_write "$HOME/.codex/packages/data"
else
    [ ! -e "$HOME/.codex/packages" ]
fi
if [ "$MEMORIES" = 1 ]; then
    [ "$(cat "$HOME/.codex/memories/data")" = memory ]
    printf updated > "$HOME/.codex/memories/data"
else
    [ ! -e "$HOME/.codex/memories" ]
fi
if [ "$AGENTS" = 1 ]; then
    [ "$(cat "$HOME/.codex/AGENTS.md")" = agents ]
    deny_write "$HOME/.codex/AGENTS.md"
else
    [ ! -e "$HOME/.codex/AGENTS.md" ]
fi
printf workspace > "$WORKSPACE/new"
[ "$(cat "$WORKSPACE/.git/config")" = git ]
printf updated > "$WORKSPACE/.git/config"
[ "$(cat "$WORKSPACE/overlay")" = overlay ]
deny_write "$WORKSPACE/overlay"
[ "$(cat "$OTHER/data")" = other ]
deny_write "$OTHER/data"
[ "$(cat "$EXTRA/data")" = exposed ]
deny_write "$EXTRA/data"
if /bin/sh -c '. "$HOME/.codex/shell_snapshots/snapshot.sh"' 2>/dev/null; then
    exit 1
fi
echo isolation-ok
"""


class IsolationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not any(os.environ.get(name) for name in (
            "BWRAP_ISOLATION_CURRENT", "BWRAP_ISOLATION_LEGACY",
        )):
            raise unittest.SkipTest("set BWRAP_ISOLATION_CURRENT or BWRAP_ISOLATION_LEGACY")
        temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(temp.cleanup)
        cls.launchers = install_both(Path(temp.name))

    def test_current_format(self):
        self.run_matrix("BWRAP_ISOLATION_CURRENT", legacy=False)

    def test_legacy_format(self):
        self.run_matrix("BWRAP_ISOLATION_LEGACY", legacy=True)

    def run_matrix(self, variable, *, legacy):
        real = os.environ.get(variable)
        if not real:
            self.skipTest(f"set {variable} to the real Bubblewrap executable")
        self.assertTrue(os.path.isabs(real) and os.path.isfile(real) and os.access(real, os.X_OK),
                        f"{variable} requires the absolute path of an executable Bubblewrap")
        for launcher in self.launchers:
            for nested in (False, True):
                for present in range(8):
                    with self.subTest(launcher=launcher.parent.name, nested=nested, present=present):
                        self.run_case(launcher, real, legacy, nested, present)

    def run_case(self, launcher, real, legacy, nested, present):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(os.path.realpath(temp))
            home = root / "home"
            workspace = home / "projects/work" if nested else root / "work"
            dot_codex = home / ".codex"
            other = root / "other"
            extra = root / "extra"
            git = workspace / ".git"
            overlay = home / ".config/bwrap-wrapper/cwd/overlay"
            files = {
                home / "private": "private",
                dot_codex / "tmp/unrelated": "secret",
                dot_codex / "tmp/arg0-linux/other/hidden": "secret",
                dot_codex / "shell_snapshots/snapshot.sh": "true",
                git / "config": "git",
                workspace / "overlay": "original",
                overlay: "overlay",
                other / "data": "other",
                extra / "data": "exposed",
                overlay.parent.parent / "config.json": json.dumps({
                    "codex": {"extra_read_only_dirs": [str(extra)]},
                }),
            }
            optional = (
                (1, "packages/data", "package"),
                (2, "memories/data", "memory"),
                (4, "AGENTS.md", "agents"),
            )
            files.update({dot_codex / name: data for bit, name, data in optional if present & bit})
            for path, data in files.items():
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(data, encoding="utf-8")

            generated = ""
            if not legacy and present & 1:
                generated_path = dot_codex / "tmp/arg0-linux/fixture"
                generated_path.mkdir()
                target = executable(dot_codex / "packages/codex", HELPER)
                helper = generated_path / "codex-linux-sandbox"
                helper.symlink_to(target)
                generated = str(generated_path)
            else:
                helper = executable(root / ("codex" if legacy else "codex-linux-sandbox"), HELPER)

            real_dir = root / "real"
            real_dir.mkdir()
            (real_dir / "bwrap").symlink_to(real)
            args = [
                "--ro-bind", "/", "/", "--dev", "/dev",
                "--ro-bind", other, other, "--tmpfs", extra,
                "--bind", workspace, workspace, "--ro-bind", git, git,
            ]
            if legacy:
                args.extend(("--argv0", "codex-linux-sandbox"))
            args.extend(("--", helper, "--sandbox-policy-cwd", workspace,
                         "--", "/bin/sh", "-eu", "-c", PROBE))
            env = {
                "PATH": f"{launcher.parent}:{real_dir}:/usr/bin:/bin",
                "HOME": str(home), "WORKSPACE": str(workspace), "OTHER": str(other),
                "EXTRA": str(extra), "GENERATED": generated, "LC_ALL": "C",
                "PACKAGES": str(present & 1), "MEMORIES": str((present >> 1) & 1),
                "AGENTS": str((present >> 2) & 1),
            }
            result = subprocess.run([launcher, *args], env=env, capture_output=True,
                                    cwd=root, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))
            self.assertEqual(result.stdout, b"isolation-ok\n")
            self.assertEqual((workspace / "new").read_bytes(), b"workspace")
            self.assertEqual((git / "config").read_bytes(), b"updated")
            self.assertEqual((workspace / "overlay").read_bytes(), b"original")
            self.assertEqual((home / "private").read_bytes(), b"private")
            for bit, name, data in optional:
                if present & bit:
                    expected = b"updated" if bit == 2 else data.encode("utf-8")
                    self.assertEqual((dot_codex / name).read_bytes(), expected)
                else:
                    self.assertFalse((dot_codex / name.partition("/")[0]).exists())
