"""JSON settings and lazy discovery of workspace overlay files."""

from dataclasses import dataclass
from decimal import Decimal
import errno
import json
import os
from pathlib import Path
import stat
from typing import Tuple


@dataclass(frozen=True)
class Settings:
    logging_enabled: bool = False
    extra_read_only_dirs: Tuple[str, ...] = ()


def _root(home, xdg_config_home):
    return Path(xdg_config_home or os.path.join(home, ".config")) / "bwrap-wrapper"


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate key {key!r}")
        result[key] = value
    return result


def _invalid_constant(value):
    raise ValueError(f"invalid JSON constant {value}")


def _object(value, name):
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _expand_home(path, home):
    first, _, suffix = path.partition("/")
    if first != "~":
        return path
    # Rust's strip_prefix skips redundant separators and dot components at
    # the ends of the suffix, preserving its interior spelling and '..'.
    parts = suffix.split("/")
    start, end = 0, len(parts)
    while start < end and parts[start] in ("", "."):
        start += 1
    while end > start and parts[end - 1] in ("", "."):
        end -= 1
    return os.path.join(home, "/".join(parts[start:end]))


def parse(contents, home):
    if not contents.strip():
        return Settings()
    # Unknown numeric settings may exceed CPython's integer digit limit.
    decoded = json.loads(contents, object_pairs_hook=_unique_object,
                         parse_constant=_invalid_constant, parse_int=Decimal)
    settings = _object(decoded, "settings")
    logging = _object(settings.get("logging", {}), "logging")
    codex = _object(settings.get("codex", {}), "codex")
    enabled = logging.get("enabled", False)
    if not isinstance(enabled, bool):
        raise ValueError("logging.enabled must be a boolean")
    paths = codex.get("extra_read_only_dirs", [])
    if not isinstance(paths, list):
        raise ValueError("codex.extra_read_only_dirs must be an array")

    expanded = []
    for path in paths:
        if isinstance(path, str):
            path.encode("utf-8")
            path = _expand_home(path, home)
            if os.path.isabs(path) and "\0" not in path:
                expanded.append(path)
                continue
        raise ValueError(
            "codex.extra_read_only_dirs requires absolute or home-relative paths without NUL bytes"
        )
    return Settings(enabled, tuple(expanded))


def load(home, xdg_config_home):
    path = _root(home, xdg_config_home) / "config.json"
    try:
        path = path.absolute()
    except OSError as error:
        raise ValueError(f"failed to resolve configuration file {path}: {error}") from error
    try:
        contents = path.read_bytes()
    except FileNotFoundError:
        return Settings()
    except OSError as error:
        raise ValueError(f"failed to read configuration file {path}: {error}") from error
    try:
        return parse(contents.decode("utf-8"), home)
    except (ValueError, RecursionError) as error:
        raise ValueError(f"invalid configuration file {path}: {error}") from error


def _matches(path, check_mode):
    try:
        return check_mode(path.stat().st_mode)
    except OSError as error:
        # Match the Rust port's pathlib-compatible metadata checks.
        if error.errno in (errno.ENOENT, errno.ENOTDIR, errno.EBADF, errno.ELOOP):
            return False
        raise


def discover(home, xdg_config_home):
    directory = _root(home, xdg_config_home).absolute() / "cwd"
    if not _matches(directory, stat.S_ISDIR):
        return []
    return [
        str(path)
        for path in sorted(directory.iterdir(), key=lambda entry: entry.name)
        if _matches(path, stat.S_ISREG)
    ]
