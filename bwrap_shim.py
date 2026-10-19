#!/usr/bin/env python3

import os
import sys
import shlex
import pwd
from pathlib import Path
from datetime import datetime


def format_args(args: list[str]) -> str:
    return " ".join(shlex.quote(arg) for arg in args)


def main() -> None:
    os.umask(0o077)

    home = os.environ["HOME"]
    config_home = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config")

    real_bwrap = os.environ.get("BWRAP_REAL", "/usr/bin/bwrap")
    log_file = os.environ.get(
        "BWRAP_LOG",
        f"{home}/.local/state/bwrap-wrapper.log",
    )

    Path(log_file).parent.mkdir(parents=True, exist_ok=True)

    timestamp = datetime.now().astimezone().isoformat(timespec="seconds")
    user_name = pwd.getpwuid(os.getuid()).pw_name
    pid = os.getpid()

    original_args = sys.argv[1:]
    # Require --argv0 codex-linux-sandbox before -- and */codex immediately after --.
    separator_index = original_args.index("--") if "--" in original_args else -1
    wrapper_args = original_args[:separator_index] if separator_index >= 0 else original_args
    is_codex_linux_sandbox = any(
        wrapper_args[i] == "--argv0"
        and wrapper_args[i + 1] == "codex-linux-sandbox"
        for i in range(len(wrapper_args) - 1)
    )
    has_codex_command = (
        separator_index >= 0
        and separator_index + 1 < len(original_args)
        and os.path.basename(original_args[separator_index + 1]) == "codex"
        and "/" in original_args[separator_index + 1]
    )
    is_codex = is_codex_linux_sandbox and has_codex_command
    args = original_args.copy() if not is_codex else []
    sandbox_cwd = None

    if is_codex:
        i = 0
        while i < len(original_args):
            arg = original_args[i]

            if arg == "--sandbox-policy-cwd":
                if i + 1 < len(original_args):
                    sandbox_cwd = original_args[i + 1]
                break

            if arg.startswith("--sandbox-policy-cwd="):
                sandbox_cwd = arg.split("=", 1)[1]
                break

            i += 1

    if is_codex and sandbox_cwd:
        sandbox_cwd = os.path.normpath(sandbox_cwd)
        if not os.path.isabs(sandbox_cwd):
            sys.exit("Error: --sandbox-policy-cwd must be absolute")
    else:
        sandbox_cwd = None

    tmpfs_inserted = False

    i = 0

    while is_codex and i < len(original_args):
        arg = original_args[i]

        if arg == "--":
            args.extend(original_args[i:])
            break

        # Remove the read-only bind mount for sandbox_cwd/.git.
        if arg == "--ro-bind" and i + 2 < len(original_args) and sandbox_cwd is not None:
            git_path = os.path.join(sandbox_cwd, ".git")
            if original_args[i + 1] == git_path and original_args[i + 2] == git_path:
                i += 3
                continue

        # Convert --dev /dev.
        if (
            arg == "--dev"
            and i + 1 < len(original_args)
            and original_args[i + 1] == "/dev"
        ):
            args.extend(["--dev-bind", "/dev", "/dev"])
            i += 2
            continue

        # Handle --bind arguments.
        if arg == "--bind" and i + 2 < len(original_args):
            src = original_args[i + 1]
            dst = original_args[i + 2]

            # If sandbox_cwd is set and
            # both SRC and DST equal sandbox_cwd:
            if (
                sandbox_cwd is not None
                and src == sandbox_cwd
                and dst == sandbox_cwd
            ):

                # Insert tmpfs only at the first matching bind mount.
                if not tmpfs_inserted:
                    args.extend([
                        "--tmpfs", home,
                        "--dir", os.path.join(home, ".codex"),
                        "--ro-bind",
                        os.path.join(home, ".codex/packages"),
                        os.path.join(home, ".codex/packages"),
                        "--bind",
                        os.path.join(home, ".codex/memories"),
                        os.path.join(home, ".codex/memories"),
                        "--ro-bind",
                        os.path.join(home, ".codex/AGENTS.md"),
                        os.path.join(home, ".codex/AGENTS.md"),
                    ])
                    tmpfs_inserted = True

            args.extend(["--bind", src, dst])
            if sandbox_cwd is not None and src == sandbox_cwd and dst == sandbox_cwd:
                cwd_config = Path(config_home).absolute() / "bwrap-wrapper" / "cwd"
                if cwd_config.is_dir():
                    for source in sorted(cwd_config.iterdir()):
                        if source.is_file():
                            args.extend([
                                "--ro-bind", str(source),
                                os.path.join(sandbox_cwd, source.name),
                            ])
            i += 3
            continue

        args.append(arg)
        i += 1

    # Write the log.
    with open(log_file, "a", encoding="utf-8") as log:
        log.write("\n========== BWRAP START ==========\n")
        log.write(f"Timestamp: {timestamp}\n")
        log.write(f"User: {user_name}\n")
        log.write(f"PID: {pid}\n")
        log.write(f"Codex invocation: {is_codex}\n")
        log.write(f"Bwrap: {real_bwrap}\n")
        log.write(f"Sandbox CWD: {sandbox_cwd or '(not specified)'}\n")
        log.write(f"Tmpfs inserted: {tmpfs_inserted}\n")

        log.write(f"Original args: {format_args(original_args)}\n")
        log.write(f"Modified args: {format_args(args)}\n")

        log.write("\nEnvironment:\n")
        for key, value in sorted(os.environ.items()):
            log.write(f"{key}={value}\n")

        log.write("=========== BWRAP END ===========\n")

    # Replace the current process, equivalent to Bash's exec.
    os.execv(real_bwrap, [real_bwrap, *args])


if __name__ == "__main__":
    main()