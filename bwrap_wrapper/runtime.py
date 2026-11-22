"""Discover the real Bubblewrap, optionally log, and replace the process."""

from contextlib import contextmanager, nullcontext
import os
import signal
import stat
import sys

from . import config, mounts, transform
from .invocation_log import InvocationLog

NATIVE_LAUNCHER_MARKER = b"\0bwrap-wrapper native launcher\0"
LEGACY_LAUNCHER_MARKER = b"# bwrap-wrapper Python launcher\n"
SCRIPT_SCAN_LIMIT = 32 * 1024


def _restore_locale_environment():
    # On Linux procfs exposes the original exec environment. CPython can
    # insert or replace LC_CTYPE before Python code begins (PEP 538).
    with open("/proc/self/environ", "rb") as proc:
        original = next(
            (item.partition(b"=")[2] for item in proc.read().split(b"\0")
             if item.startswith(b"LC_CTYPE=")),
            None,
        )
    if original != os.environb.get(b"LC_CTYPE"):
        if original is None:
            del os.environb[b"LC_CTYPE"]
        else:
            os.environb[b"LC_CTYPE"] = original


def _is_wrapper_launcher(path):
    try:
        # A candidate can be replaced after stat. Avoid blocking if it becomes a FIFO.
        with os.fdopen(os.open(path, os.O_RDONLY | os.O_NONBLOCK), "rb") as candidate:
            header = candidate.read(4)
            if header.startswith(b"#!"):
                script = header + candidate.read(SCRIPT_SCAN_LIMIT - len(header))
                if script.partition(b"\n")[2].startswith(LEGACY_LAUNCHER_MARKER):
                    return True
                # uv and pip use the same import and call, even with a shell prefix.
                lines = {line.strip() for line in script.splitlines()}
                return (b"from bwrap_wrapper.runtime import main" in lines
                        and b"sys.exit(main())" in lines)
            if header == b"\x7fELF":
                candidate.seek(-len(NATIVE_LAUNCHER_MARKER), os.SEEK_END)
                return candidate.read(len(NATIVE_LAUNCHER_MARKER)) == NATIVE_LAUNCHER_MARKER
    except OSError:
        pass
    return False


def find_bwrap(launcher):
    try:
        own = os.stat(launcher)
    except OSError as error:
        raise ValueError(f"failed to identify current executable: {error}") from error
    path = os.environ.get("PATH")
    if path is None:
        raise ValueError("PATH is not set; cannot find bwrap")
    for directory in path.split(os.pathsep):
        candidate = os.path.join(directory, "bwrap")
        try:
            metadata = os.stat(candidate)
        except OSError:
            continue
        if (stat.S_ISREG(metadata.st_mode) and not os.path.samestat(metadata, own)
                and os.access(candidate, os.X_OK) and not _is_wrapper_launcher(candidate)):
            return candidate
    raise ValueError("no executable bwrap other than this wrapper found in PATH")


@contextmanager
def _project_tmp_fd(home, workspace):
    cache = os.path.join(home, ".cache")
    parts = ("codex", "tmp", *filter(None, workspace.split("/")))
    flags = os.O_PATH | os.O_DIRECTORY
    fd = None
    try:
        try:
            os.makedirs(cache, mode=0o700, exist_ok=True)
            fd = os.open(cache, flags)
            for part in parts:
                try:
                    os.mkdir(part, mode=0o700, dir_fd=fd)
                except FileExistsError:
                    pass
                # A parent project's /tmp can contain another project's cache.
                child = os.open(part, flags | os.O_NOFOLLOW, dir_fd=fd)
                os.close(fd)
                fd = child
            os.set_inheritable(fd, True)
        except OSError as error:
            path = os.path.join(cache, *parts)
            raise ValueError(f"failed to prepare project /tmp at {path}: {error}") from error
        yield fd
    finally:
        if fd is not None:
            os.close(fd)


def run(original, launcher, read_mounts=mounts.read_file):
    os.umask(0o077)
    _restore_locale_environment()
    home = os.environ.get("HOME")
    if home is None:
        raise ValueError("HOME environment variable is required")
    xdg = os.environ.get("XDG_CONFIG_HOME")
    settings = config.load(home, xdg)

    log = None
    if settings.logging_enabled:
        path = os.environ.get("BWRAP_LOG", home + "/.local/state/bwrap-wrapper.log")
        try:
            log = InvocationLog(path)
        except (OSError, KeyError, ValueError) as error:
            raise ValueError(f"failed to prepare invocation log: {error}") from error

    output = transform.transform(original, home, lambda: config.discover(home, xdg),
                                 read_mounts, settings.extra_read_only_dirs)
    real_bwrap = find_bwrap(launcher)
    index = output.project_tmp_index
    project_tmp = _project_tmp_fd(home, output.workspace) if index is not None else nullcontext()
    with project_tmp as fd:
        if fd is not None:
            output.args[index:index] = ("--bind-fd", str(fd), "/tmp")
        if log is not None:
            try:
                log.append(real_bwrap, original, output)
            except (OSError, UnicodeError) as error:
                raise ValueError(f"failed to write invocation log: {error}") from error

        signal.signal(signal.SIGPIPE, signal.SIG_IGN)
        signal.signal(signal.SIGXFSZ, signal.SIG_DFL)
        try:
            os.execv(real_bwrap, [real_bwrap, *output.args])
        except (OSError, ValueError) as error:
            raise ValueError(f"failed to execute {real_bwrap}: {error}") from error


def main():
    try:
        run(sys.argv[1:], sys.argv[0])
    except (OSError, ValueError) as error:
        print(f"bwrap-wrapper: {error}", file=sys.stderr)
        raise SystemExit(1) from None
