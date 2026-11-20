# Installation and usage

The wrapper requires Linux or WSL2, CPython 3.9 or newer, and a working
Bubblewrap. Use `uv` to install it. Source builds require `cc` or the C
compiler named by `CC`.

## Installation

Run these commands from the project root.

```sh
uv tool install .
export PATH="$(uv tool dir --bin):$PATH"
command -v bwrap
```

`command -v bwrap` should show `bwrap` in the uv tool bin directory. Keep
the real Bubblewrap later in `PATH`, such as `/usr/bin/bwrap`. You can then
run `bwrap` from any working directory. Pass `--python 3.14` to `uv tool install`
to select a specific CPython version. [uv tool installation](https://docs.astral.sh/uv/guides/tools/#installing-tools)

The launcher starts Python from the tool environment in isolated mode with
site initialization disabled and UTF-8 filesystem decoding. The wrapper
picks the first executable regular file named `bwrap` on `PATH` after skipping
links to itself and marked wrapper installations, including previous releases.
Relative and empty entries are resolved from the working directory. An unset
`PATH` or a search with no suitable candidate causes an error.

## Settings

The wrapper reads UTF-8 JSON from
`${XDG_CONFIG_HOME:-$HOME/.config}/bwrap-wrapper/config.json`. Relative config
roots are anchored to the working directory, retaining `..` and symlinks.

```json
{
  "logging": {
    "enabled": false
  },
  "codex": {
    "extra_read_only_dirs": ["~/reference", "/opt/shared"]
  }
}
```

A missing or whitespace-only file uses the defaults. The top level, `logging`,
and `codex` must be objects. Logging defaults to `false` and takes a JSON
boolean. The extra directory list defaults to an empty array. Each entry
must be a string containing an absolute path, `~`, or a path starting with
`~/`, without NUL bytes. Tilde expansion uses the caller's `HOME`, including
the trailing slash produced by bare `~`. The expansion must yield an absolute
path. The wrapper ignores unknown keys and rejects duplicate keys, invalid
JSON, and invalid values for supported settings.

| Variable | Behavior |
| --- | --- |
| `HOME` | Must be set. Supplies the HOME mount path, tilde expansion, and default settings and log locations. |
| `XDG_CONFIG_HOME` | An unset or empty value uses `$HOME/.config`. |
| `PATH` | Supplies the ordered executable search. An empty value searches the working directory. |
| `BWRAP_LOG` | Selects the log path when logging is enabled. An unset value uses `$HOME/.local/state/bwrap-wrapper.log`. |

## Codex detection and workspace mounts

The legacy format pairs `--argv0 codex-linux-sandbox` before the first
`--` with a command path containing `/` and ending in `/codex`. It scans
the full argument list for the first `--sandbox-policy-cwd PATH` or
`--sandbox-policy-cwd=PATH`. The current format has a command with the
exact basename `codex-linux-sandbox` immediately after the first `--`.
It scans only the helper options before the nested command separator.
A dangling `--argv0` immediately before the first separator suppresses the
current-format detector.

A missing or empty workspace value leaves the workspace unspecified. A nonempty
value is normalized lexically and must be absolute. Mount operands retain their
original spelling and are compared byte for byte. Other invocations keep their
original arguments.

The wrapper applies the following changes to recognized invocations, preserving
their original mount sequence around the changes.

1. Replace `--dev /dev` with `--dev-bind /dev /dev`.
2. Remove complete protections at the exact `.git`, `.agents`, `.codex`, and
   `.aws` paths directly under the workspace and at the fixed paths directly
   under `/tmp`. A protection is a self `--ro-bind P P` or a pair
   `--tmpfs P --remount-ro P` with an optional adjacent `--perms MODE` prefix.
   Incomplete groups and other mounts are preserved. For example, the Codex
   daemon directory mask remains in place.
3. Before the first exact `--bind WORKSPACE WORKSPACE`, mount tmpfs on HOME,
   create `HOME/.codex`, and re-expose packages and `AGENTS.md` read-only and
   memories writable. These three mounts use Bubblewrap's `--ro-bind-try` and
   `--bind-try` options to skip missing sources. If a current-format command
   names exactly
   `HOME/.codex/tmp/arg0-linux/<directory>/codex-linux-sandbox`,
   create the two empty helper parent directories in the sandbox and bind
   only that immediate directory read-only. The helper path is checked
   lexically and can be a symlink into the packages mount. Unrelated helper
   directories and shell snapshots remain hidden in this mount layout.
4. After every matching workspace bind, re-read
   `${XDG_CONFIG_HOME:-$HOME/.config}/bwrap-wrapper/cwd/`. Bind each direct
   regular file read-only over `WORKSPACE/<filename>`, in filename order
   after UTF-8 decoding with surrogateescape. Hidden files and symlinks to
   regular files are included. Missing directories, non-file entries, and
   broken links add no mounts. Other discovery errors abort execution.
5. At the command separator, read `/proc/mounts` as UTF-8 and mask each WSL2
   Windows drive with `--tmpfs MOUNTPOINT`. The filesystem type must be
   exactly `9p` and the comma- or semicolon-delimited options must include
   exactly `aname=drvfs`. Decode three-digit octal escapes and deduplicate.
   Masks are ordered by slash count and then by name. After those masks,
   bind the configured extra directories read-only at the same paths using
   `--ro-bind-try` in configuration order.

The loop preserves the separator and command tail when it reaches the
separator. A malformed `--bind` still consumes two operands when available,
including a separator in either operand position. Mount discovery runs at
the separator the loop actually reaches. A call with no matching workspace
bind can still get the drive masks and configured mounts.

The original workspace binds remain writable, and removing an exact protection
allows the surrounding mount to supply access to that path. A later bind can
change visibility again. The wrapper relies on Bubblewrap to enforce the
forwarded mount sequence.

## Logging and execution

Logging is off by default. Set `logging.enabled` to `true` to append one
entry per invocation before execution. `BWRAP_LOG` overrides the destination.
The process sets umask `0077`. New directories and log files normally have
modes `0700` and `0600`, respectively. Existing permissions are preserved.

Entries include local timestamp, username, process ID, Codex detection,
workspace, selected Bubblewrap path, original and modified arguments, and
the entire environment sorted by variable name. Arguments use POSIX shell
quoting. **Logs can contain credentials and other sensitive data.** Protect
the log and redact it before sharing.

The wrapper validates settings before creating any log artifacts. With logging
enabled, it prepares the directory before rewriting, then appends the entry
after executable discovery. Logging uses strict UTF-8, so a non-UTF-8 argument,
path, or environment entry can leave a partial entry and prevent execution.
With logging disabled, Unix argument and environment bytes pass through.

The wrapper uses `os.execv()` so the selected executable inherits the PID,
environment, and standard streams and supplies the exit status or signal
termination. On Linux it uses the original environment from
`/proc/self/environ` to undo changes to `LC_CTYPE` made by CPython at startup.
It keeps `SIGPIPE` ignored and restores the default `SIGXFSZ` disposition
before execution. CPython overwrites `SIGXFSZ` at startup, so the launcher
cannot recover a caller's deliberately ignored `SIGXFSZ` disposition.
[CPython locale coercion](https://peps.python.org/pep-0538/) ·
[CPython signal initialization](https://raw.githubusercontent.com/python/cpython/3.9/Modules/signalmodule.c)
