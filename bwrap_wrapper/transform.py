"""Codex detection and argument rewriting, with injectable mount discovery."""

from dataclasses import dataclass
import os
from typing import List, Optional

HELPER = "codex-linux-sandbox"
POLICY_CWD = "--sandbox-policy-cwd"
PROTECTED = (".git", ".agents", ".codex", ".aws")
BIND_OPTIONS = (
    "--bind", "--bind-try", "--ro-bind", "--ro-bind-try",
    "--dev-bind", "--dev-bind-try", "--bind-fd", "--ro-bind-fd",
)


@dataclass
class Output:
    args: List[str]
    is_codex: bool
    workspace: Optional[str] = None
    home_inserted: bool = False
    project_tmp_index: Optional[int] = None


def _helper_options(options, command):
    if not command or (options and options[-1] == "--argv0"):
        return None
    if os.path.basename(command[0]) != HELPER:
        return None
    helper_args = command[1:]
    return helper_args[:helper_args.index("--")] if "--" in helper_args else helper_args


def _is_legacy(options, command):
    return (
        bool(command)
        and "/" in command[0]
        and os.path.basename(command[0]) == "codex"
        and any(pair == ("--argv0", HELPER) for pair in zip(options, options[1:]))
    )


def _workspace(args):
    for index, arg in enumerate(args):
        if arg == POLICY_CWD:
            value = args[index + 1] if index + 1 < len(args) else ""
            break
        if arg.startswith(POLICY_CWD + "="):
            value = arg[len(POLICY_CWD) + 1:]
            break
    else:
        return None
    if not value:
        return None
    workspace = os.path.normpath(value)
    if not os.path.isabs(workspace):
        raise ValueError("Error: --sandbox-policy-cwd must be absolute")
    return workspace


def _helper_directory(command, home):
    if not home.startswith("/") or any(p in ("", ".", "..") for p in home.split("/")[1:]):
        return None
    prefix = os.path.join(home, ".codex/tmp/arg0-linux/")
    if command.startswith(prefix):
        directory, _, executable = command[len(prefix):].partition("/")
        if directory not in ("", ".", "..") and executable == HELPER:
            return prefix + directory
    return None


def _home_mounts(home, helper):
    args = ["--tmpfs", home, "--dir", os.path.join(home, ".codex")]
    for option, name in (
        ("--ro-bind-try", "packages"),
        ("--bind-try", "memories"),
        ("--ro-bind-try", "AGENTS.md"),
    ):
        path = os.path.join(home, ".codex", name)
        args.extend((option, path, path))
    if helper is not None:
        args.extend(("--dir", os.path.join(home, ".codex/tmp"),
                     "--dir", os.path.join(home, ".codex/tmp/arg0-linux"),
                     "--ro-bind", helper, helper))
    return args


def _protection_length(options, index, targets):
    mount = index + 2 if options[index] == "--perms" else index
    if mount == index:
        group = options[mount:mount + 3]
        if len(group) == 3 and group[0] == "--ro-bind" and group[1] == group[2] in targets:
            return 3
    group = options[mount:mount + 4]
    if (len(group) == 4 and group[0] == "--tmpfs" and group[2] == "--remount-ro"
            and group[1] == group[3] in targets):
        return mount - index + 4
    return 0


def transform(args, home, read_overlays, read_mounts, extra_read_only_dirs=()):
    args = list(args)
    if "--" not in args:
        return Output(args, False)
    end = args.index("--")
    options, command = args[:end], args[end + 1:]
    helper_options = _helper_options(options, command)
    if helper_options is None and not _is_legacy(options, command):
        return Output(args, False)

    workspace = _workspace(args if helper_options is None else helper_options)
    bases = ("/tmp",) if workspace is None else ("/tmp", workspace)
    targets = {os.path.join(base, name) for base in bases for name in PROTECTED}
    output = Output([], True, workspace)
    forwarded = output.args

    tmp_index = 0
    tmp_base_end = 0
    i = 0
    while i < len(args):
        arg = args[i]
        if arg == "--":
            for mount in read_mounts():
                forwarded.extend(("--tmpfs", mount))
            for path in extra_read_only_dirs:
                forwarded.extend(("--ro-bind-try", path, path))
            forwarded.extend(args[i:])
            break
        protection = _protection_length(options, i, targets) if i < end else 0
        if protection:
            i += protection
        elif arg == "--perms" and i + 1 < end:
            # Keep an unmatched modifier and its operand together.
            forwarded.extend(args[i:i + 2])
            i += 2
        elif arg == "--dev" and i + 1 < len(args) and args[i + 1] == "/dev":
            forwarded.extend(("--dev-bind", "/dev", "/dev"))
            i += 2
        elif ((arg in BIND_OPTIONS and i + 2 < end)
              or (arg == "--bind" and i + 2 < len(args))):
            # Even a malformed bind consumes both operands, possibly the separator.
            source, target = args[i + 1:i + 3]
            matching = arg == "--bind" and source == target == workspace
            if matching and not output.home_inserted:
                helper = _helper_directory(command[0], home) if helper_options is not None else None
                forwarded.extend(_home_mounts(home, helper))
                output.home_inserted = True
            host_tmp = arg in ("--bind", "--bind-try") and source == target == "/tmp"
            if workspace is not None and host_tmp:
                tmp_index = tmp_base_end
            else:
                forwarded.extend((arg, source, target))
                if i + 2 < end and target in ("/", "/tmp"):
                    tmp_base_end = len(forwarded)
                    if target == "/":
                        tmp_index = tmp_base_end
            if matching:
                for overlay in read_overlays():
                    dest = os.path.join(workspace, os.path.basename(overlay))
                    forwarded.extend(("--ro-bind", overlay, dest))
            i += 3
        elif arg == "--tmpfs" and i + 1 < end and args[i + 1] == "/":
            forwarded.extend((arg, "/"))
            tmp_index = tmp_base_end = len(forwarded)
            i += 2
        else:
            forwarded.append(arg)
            i += 1

    if workspace is not None:
        # Mount before /tmp descendants so workspace binds and masks stay visible.
        output.project_tmp_index = tmp_index
    return output
