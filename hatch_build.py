"""Build the native executable shipped in the wheel's scripts directory."""

import os
from pathlib import Path
import shlex
import subprocess
import sys
import sysconfig
from tempfile import TemporaryDirectory

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        if version != "standard":
            raise RuntimeError("Install the wrapper with uv tool install .")
        if sys.platform != "linux" or sys.implementation.name != "cpython":
            raise RuntimeError("bwrap-wrapper requires CPython on Linux or WSL2")

        compiler = shlex.split(os.environ.get("CC") or "cc")
        self.launcher_dir = TemporaryDirectory(prefix="bwrap-launcher-")
        launcher = Path(self.launcher_dir.name) / "bwrap"
        source = Path(self.root) / "launcher.c"
        try:
            subprocess.run([*compiler, "-std=c99", "-O2", str(source), "-o", str(launcher)],
                           check=True)
        except FileNotFoundError as error:
            raise RuntimeError("Building bwrap-wrapper requires a C compiler (CC or cc)") from error

        # The Linux ELF loader ignores this trailer. Keep it in sync with runtime.py.
        with launcher.open("ab") as binary:
            binary.write(b"\0bwrap-wrapper native launcher\0")

        platform = sysconfig.get_platform().replace("-", "_").replace(".", "_")
        build_data["tag"] = f"py3-none-{platform}"
        build_data["pure_python"] = False
        build_data["shared_scripts"][str(launcher)] = "bwrap"

    def finalize(self, version, build_data, artifact_path):
        self.launcher_dir.cleanup()
