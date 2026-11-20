import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from build_zipapp import build

ROOT = Path(__file__).resolve().parents[1]
PYTHON = sys.executable


def executable(path, contents):
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)
    return path


def install_source(output):
    output.mkdir()
    shutil.copytree(ROOT / "bwrap_wrapper", output / "bwrap_wrapper",
                    ignore=shutil.ignore_patterns("__pycache__"))
    _, body = (ROOT / "bwrap").read_bytes().split(b"\n", 1)
    source = output / "bwrap"
    source.write_bytes(os.fsencode(f"#!{PYTHON} -ISBXutf8\n") + body)
    source.chmod(0o755)
    return source


def install_both(output):
    source = install_source(output / "source")
    archive = build(output / "archive", PYTHON)
    return source, archive


class FakeRuntime:
    def __init__(self, root):
        self.root = root
        self.home = root / "home"
        self.settings = self.home / ".config/bwrap-wrapper/config.json"
        self.log = root / "state/invocation.log"
        self.real = root / "real"
        self.real.mkdir()
        self.fake = executable(self.real / "bwrap", f"""#!{PYTHON} -ISBXutf8
import os
from pathlib import Path
import signal
import sys

if os.environ.get("FAKE_PID_FILE"):
    Path(os.environ["FAKE_PID_FILE"]).write_text(str(os.getpid()), encoding="ascii")
if os.environ.get("FAKE_ENV"):
    Path(os.environ["FAKE_ENV"]).write_bytes(Path("/proc/self/environ").read_bytes())
sys.stdout.buffer.write(b"".join(os.fsencode(arg) + b"\\0" for arg in sys.argv[1:]))
sys.stdout.buffer.flush()
if os.environ.get("FAKE_STREAMS"):
    sys.stdout.buffer.write(sys.stdin.buffer.read())
    sys.stderr.buffer.write(b"fake stderr\\n")
if os.environ.get("FAKE_SIGNAL"):
    os.kill(os.getpid(), int(os.environ["FAKE_SIGNAL"]))
sys.exit(int(os.environ.get("FAKE_EXIT", 0)))
""")
        self.env = {"HOME": str(self.home), "PATH": str(self.real), "BWRAP_LOG": str(self.log)}

    def config(self, contents):
        self.settings.parent.mkdir(parents=True, exist_ok=True)
        self.settings.write_bytes(contents)

    def enable_logging(self):
        self.config(json.dumps({"logging": {"enabled": True}}).encode("utf-8"))

    def command(self, launcher, args=(), env=None, **kwargs):
        return subprocess.run([*launcher, *args], env=self.env if env is None else env,
                              cwd=self.root, capture_output=True, timeout=15, **kwargs)
