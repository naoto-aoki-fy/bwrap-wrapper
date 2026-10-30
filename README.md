# bwrap-wrapper

A Python wrapper for bubblewrap that adjusts Codex sandbox arguments, logs each
invocation, and then replaces itself with the real `bwrap` process. Other
invocations are logged and forwarded with their arguments unchanged.

## Installation

Requires Python 3.9 or newer and bubblewrap (normally `/usr/bin/bwrap`).

```sh
python3 -m venv .venv
.venv/bin/python -m pip install .
PATH="$PWD/.venv/bin:$PATH" codex
```

The package installs a `bwrap` command. Putting the virtual environment's `bin`
directory first in `PATH` lets Codex find the shim. Keep `BWRAP_REAL` pointed at
the actual bubblewrap executable to avoid recursion.

You can also invoke the script directly:

```sh
python3 bwrap_shim.py --ro-bind / / -- /bin/true
```

## Codex argument changes

The shim recognizes Codex when arguments before `--` include
`--argv0 codex-linux-sandbox` and the command immediately after `--` is a path
containing `/` whose basename is `codex`.

For these invocations, it:

- Converts `--dev /dev` to `--dev-bind /dev /dev`.
- Hides WSL2 Windows drives detected as `9p` mounts with `aname=drvfs` in
  `/proc/mounts` by mounting tmpfs over each mount point after the other mounts.
- Reads `--sandbox-policy-cwd PATH` or `--sandbox-policy-cwd=PATH`, normalizes the
  path, and requires it to be absolute when nonempty.
- Removes `--ro-bind PATH/.git PATH/.git` for that workspace.
- Before the first `--bind PATH PATH` for the workspace, inserts a tmpfs over
  `$HOME`, creates `$HOME/.codex`, mounts `$HOME/.codex/packages` read-only, and
  mounts `$HOME/.codex/memories` writable, and mounts `$HOME/.codex/AGENTS.md`
  read-only at the same path. The workspace bind is retained.
- Mounts each file directly inside `$XDG_CONFIG_HOME/bwrap-wrapper/cwd` read-only
  at `PATH/<filename>`, including hidden files. If `XDG_CONFIG_HOME` is unset or
  empty, uses `$HOME/.config`. Directories are skipped; a missing configuration
  directory adds no mounts. These mounts are inserted immediately after each
  workspace `--bind PATH PATH`, overlaying workspace files with the same names.

The home mounts require the corresponding source paths to exist. Without
a workspace path, the `.git` removal and home mount insertion do not occur.
These changes expose the host's `/dev`, remove the workspace `.git` read-only
overlay, and permit writes to Codex memories.

## Configuration and logging

| Variable | Default | Purpose |
| --- | --- | --- |
| `BWRAP_REAL` | `/usr/bin/bwrap` | Real bubblewrap executable |
| `BWRAP_LOG` | `$HOME/.local/state/bwrap-wrapper.log` | Append-only invocation log |
| `HOME` | Required from the environment | Home mount paths and default log location |
| `XDG_CONFIG_HOME` | `$HOME/.config` | Base directory for `bwrap-wrapper/cwd` files |

The shim creates the log's parent directories and sets a restrictive `0077`
umask. Each entry records the timestamp, user, PID, Codex detection, workspace
path, original and modified arguments, and the entire environment. Logs can
contain credentials and other sensitive values; protect them and redact them
before sharing. Existing log-file permissions are not changed, and logs are not
rotated automatically.

The real bubblewrap executable receives the arguments through `os.execv`, so
its exit status becomes the command's exit status. A logging or configuration
error prevents bubblewrap from being launched.
