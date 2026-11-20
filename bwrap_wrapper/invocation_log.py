"""Append the Rust wrapper's invocation log format using strict UTF-8."""

from datetime import datetime
import os
from pathlib import Path
import pwd
import shlex


class InvocationLog:
    def __init__(self, path):
        self.path = path
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        self.timestamp = datetime.now().astimezone().isoformat(timespec="seconds")
        self.username = pwd.getpwuid(os.getuid()).pw_name

    def append(self, real_bwrap, original, output):
        with open(self.path, "a", encoding="utf-8") as log:
            log.write("\n========== BWRAP START ==========\n")
            log.write(f"Timestamp: {self.timestamp}\n")
            log.write(f"User: {self.username}\n")
            log.write(f"PID: {os.getpid()}\n")
            log.write(f"Codex invocation: {output.is_codex}\n")
            log.write(f"Bwrap: {real_bwrap}\n")
            log.write(f"Sandbox CWD: {output.workspace or '(not specified)'}\n")
            log.write(f"Tmpfs inserted: {output.home_inserted}\n")
            log.write(f"Original args: {shlex.join(original)}\n")
            log.write(f"Modified args: {shlex.join(output.args)}\n")
            log.write("\nEnvironment:\n")
            for key, value in sorted(os.environ.items()):
                log.write(f"{key}={value}\n")
            log.write("=========== BWRAP END ===========\n")
