#!/usr/bin/env python3
"""Development-only differential check. No Bubblewrap or host mount table is used."""
import importlib.util
import io
import json
import os
from pathlib import Path
import random
import shlex
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def python_child(reference, arguments):
    spec = importlib.util.spec_from_file_location("reference", reference)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    original_open = open

    def fixture_open(path, *args, **kwargs):
        if path == "/proc/mounts":
            return io.StringIO(Path(os.environ["BWRAP_TEST_MOUNTS"]).read_text(encoding="utf-8"))
        return original_open(path, *args, **kwargs)

    module.open = fixture_open
    sys.argv = ["bwrap", *arguments]
    module.main()


def fixture_binary():
    built = subprocess.run(
        ["cargo", "test", "--test", "runtime", "--no-run", "--message-format=json"],
        cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True,
    )
    for line in built.stdout.splitlines():
        message = json.loads(line)
        if message.get("target", {}).get("name") == "runtime" and message.get("executable"):
            return message["executable"]
    raise RuntimeError("Cargo did not report the runtime test executable")


def normalize_optional_mounts(forwarded, log, home):
    """Map only the exact inserted HOME sequence back to the reference policy."""
    old = ["--tmpfs", home, "--dir", os.path.join(home, ".codex")]
    new = list(old)
    for option, suffix in [("--ro-bind", "packages"), ("--bind", "memories"), ("--ro-bind", "AGENTS.md")]:
        path = os.path.join(home, ".codex", suffix)
        old.extend([option, path, path])
        new.extend([option + "-try", path, path])
    def encoded(args):
        return b"".join(os.fsencode(arg) + b"\0" for arg in args)
    if forwarded is not None:
        forwarded = forwarded.replace(encoded(new), encoded(old), 1)
    if log is not None:
        prefix = "Modified args: "
        log = [prefix + line[len(prefix):].replace(shlex.join(new), shlex.join(old), 1)
               if line.startswith(prefix) else line for line in log]
    return forwarded, log


def main():
    reference = ROOT / "python/bwrap_shim.py"
    if not reference.is_file():
        raise SystemExit("Place the reference checkout at ./python (development check only)")
    driver = fixture_binary()
    rng = random.Random(614)
    header = ["--argv0", "codex-linux-sandbox"]
    tail = ["--", "/opt/codex"]
    cases = [
        [], [""], ["spaces here", "a'b", 'double"quote', "日本語", "λ", "line\nbreak"],
        header + ["--sandbox-policy-cwd=/w/./a/../", "--bind", "/w", "/w", "--bind", "/w", "/w"] + tail,
        header + ["--sandbox-policy-cwd=/w", "--ro-bind", "/w/.git", "/w/.git", "--ro-bind", "/w/./.git", "/w/./.git", "--ro-bind", "/w/.git", "/w/.git"] + tail,
        header + ["--bind", "source"] + tail,
        header + tail + ["--sandbox-policy-cwd=/w", "--dev", "/dev"],
        header + ["--sandbox-policy-cwd=//w///", "--bind", "//w", "//w"] + tail,
        header + tail + ["--sandbox-policy-cwd"],
        header + ["--sandbox-policy-cwd", ""] + tail,
        header + ["--sandbox-policy-cwd=", "--sandbox-policy-cwd=/later"] + tail,
        ["--argv0=codex-linux-sandbox", "--dev", "/dev"] + tail,
        header + ["--", "codex"], header + ["--", "/opt/codex/"],
        header + ["--sandbox-policy-cwd=relative"] + tail,
        [os.fsdecode(b"raw\xff")],
    ]
    workspace_case = header + ["--sandbox-policy-cwd=/w", "--bind", "/w", "/w"] + tail
    cases[6:6] = [workspace_case, workspace_case]
    cases[0:0] = [workspace_case] * 10
    tokens = ["--dev", "/dev", "--bind", "/w", "--ro-bind", "/w/.git", "--", "--argv0", "codex-linux-sandbox", "/bin/codex", "--sandbox-policy-cwd", "--sandbox-policy-cwd=/w", "--sandbox-policy-cwd=", "relative", "--sandbox-policy-cwd=//a/../w", "/w/./", "", "/bin/true", "--other", "日本語", "a'b"]
    for _ in range(300):
        before = [rng.choice(tokens) for _ in range(rng.randrange(20))]
        after = [rng.choice(tokens) for _ in range(rng.randrange(8))]
        cases.append((header if rng.random() < .8 else []) + before + tail + after)
    with tempfile.TemporaryDirectory(prefix="bwrap-reference-") as temporary:
        root = Path(temporary)
        config = root / "xdg/bwrap-wrapper/cwd"
        config.mkdir(parents=True)
        for name in [".hidden", "A", "z", "日本語"]:
            (config / name).write_text("overlay")
        (config / "directory").mkdir()
        (config / "linked-file").symlink_to("A")
        (config / "linked-directory").symlink_to("directory")
        (config / "broken").symlink_to("absent")
        mounts = root / "mounts"
        mounts.write_text("x /mnt/c 9p rw,aname=drvfs\nx /mnt/c 9p aname=drvfs\nx /mnt/c/deep/a 9p rw;aname=drvfs\rx /mnt/d\\040space 9p aname=drvfs\nx /z\\134path 9p aname=drvfs\nx /not-windows ext4 aname=drvfs\n", encoding="utf-8")
        fake = root / "fake"
        fake.write_text('#!/bin/sh\nprintf "%s\\0" "$@" > "$BWRAP_TEST_RESULT"\nexit 0\n')
        fake.chmod(0o755)
        log = root / "state/log"
        result = root / "result"
        args_file = root / "args"
        environment = {
            "HOME": str(root / "home"), "BWRAP_REAL": str(fake), "BWRAP_LOG": str(log),
            "XDG_CONFIG_HOME": str(root / "xdg"), "PATH": "/usr/bin:/bin", "TZ": "UTC0",
            "LC_ALL": "C.UTF-8", "PYTHONUTF8": "1", "PYTHONDONTWRITEBYTECODE": "1",
            "BWRAP_TEST_ARGS": str(args_file), "BWRAP_TEST_MOUNTS": str(mounts),
            "BWRAP_TEST_RESULT": str(result), "A_MARKER": "value with spaces", "Z_MARKER": "λ",
        }
        (root / "empty/bwrap-wrapper/cwd").mkdir(parents=True)
        default_config = root / "home/.config/bwrap-wrapper/cwd"
        default_config.mkdir(parents=True)
        (default_config / ".default").write_text("fallback overlay")
        cases.append(workspace_case)
        cases.extend([["exec failure"], ["log failure"], ["missing HOME"]])
        for index, arguments in enumerate(cases):
            environment_case = dict(environment)
            variant = index % 10
            if variant == 1:
                environment_case["XDG_CONFIG_HOME"] = ""
            elif variant == 2:
                environment_case.pop("XDG_CONFIG_HOME")
            elif variant == 3:
                environment_case["XDG_CONFIG_HOME"] = str(root / "missing")
            elif variant == 4:
                environment_case["XDG_CONFIG_HOME"] = str(root / "empty")
            elif variant == 5:
                environment_case["XDG_CONFIG_HOME"] = os.path.relpath(root / "xdg", ROOT)
            elif variant == 6:
                environment_case["HOME"] = ""
            elif variant == 7:
                environment_case["HOME"] += "/"
            elif variant == 8:
                environment_case["XDG_CONFIG_HOME"] = "/" + str(root / "xdg")
            elif variant == 9:
                environment_case["XDG_CONFIG_HOME"] = str(root) + "//xdg/./"
            if index == len(cases) - 4:
                environment_case["XDG_CONFIG_HOME"] = str(root / ("x" * 300))
            elif index == len(cases) - 3:
                environment_case["BWRAP_REAL"] = ""
            elif index == len(cases) - 2:
                environment_case["BWRAP_LOG"] = ""
            elif index == len(cases) - 1:
                environment_case.pop("HOME")
            args_file.write_bytes(b"".join(os.fsencode(arg) + b"\0" for arg in arguments))
            observed = []
            for command in [
                [driver, "--exact", "fixture_driver", "--nocapture"],
                [sys.executable, str(Path(__file__).resolve()), "--python-child", str(reference), *arguments],
            ]:
                log.unlink(missing_ok=True)
                result.unlink(missing_ok=True)
                outcome = subprocess.run(command, env=environment_case, cwd=ROOT, capture_output=True)
                forwarded = result.read_bytes() if result.exists() else None
                normalized_log = None
                if outcome.returncode == 0:
                    normalized_log = [line for line in log.read_text().splitlines()
                                      if not line.startswith(("Timestamp: ", "PID: "))]
                if not observed:
                    forwarded, normalized_log = normalize_optional_mounts(
                        forwarded, normalized_log, environment_case.get("HOME", ""))
                observed.append((outcome.returncode == 0, forwarded, normalized_log))
            if observed[0] != observed[1]:
                raise AssertionError(f"Case {index}: {arguments!r}\nRust: {observed[0]!r}\nPython: {observed[1]!r}")
    print(f"{len(cases)} cases matched after documented optional-HOME-mount normalization: status, forwarded bytes, and complete logs (except timestamp/PID)")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--python-child":
        python_child(sys.argv[2], sys.argv[3:])
    else:
        main()
