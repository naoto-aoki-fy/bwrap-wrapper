# Usage and behavior

## Build and installation

Supported targets are Linux and WSL2 Linux distributions. Use a stable Rust
compiler supporting edition 2024 (1.85+), Cargo, and a system linker. Cargo
dependencies include `libc`, Serde, and a TOML parser. Actual sandboxing needs a
working Bubblewrap and the host's normal user/mount namespace support. Ordinary
runtime tests use fake executables; the separate [real isolation suite](isolation.md)
verifies effective mounts with Bubblewrap.

From the repository root:

```sh
cargo build --release
test -x /usr/bin/bwrap
install -Dm755 target/release/bwrap "$HOME/.local/lib/bwrap-wrapper/bin/bwrap"
export PATH="$HOME/.local/lib/bwrap-wrapper/bin:$PATH"
codex
```

Keep the original Bubblewrap in PATH and install the wrapper in a separate
directory. The wrapper searches PATH in order for an executable regular file
named `bwrap`, skipping itself, including symlinks and hard links to itself.
Missing, broken, directory, and non-executable candidates are skipped. Relative
and empty PATH entries are resolved from the working directory (empty means the
current directory). An unset PATH or no suitable candidate causes a nonzero error.
`BWRAP_REAL` is no longer used. Persist the PATH export in your shell configuration
if desired. The wrapper binary is `target/release/bwrap`; the Cargo package is
`bwrap-wrapper-rs`.

The wrapper preserves PID, environment, standard streams, exit status, and
signal termination by replacing the process. Execution failure exits nonzero.

## Environment

| Variable | Unset default | Empty value |
| --- | --- | --- |
| `HOME` | Required; absence fails even for non-Codex calls | Accepted, as in Python |
| `PATH` | Required to find Bubblewrap | Searches the current directory |
| `BWRAP_LOG` | `$HOME/.local/state/bwrap-wrapper.log` when logging is enabled | Ignored when disabled; logging fails when enabled |
| `XDG_CONFIG_HOME` | `$HOME/.config` | Uses `$HOME/.config` |

No missing mount source is automatically created. Missing optional packages,
memories, and AGENTS.md sources are skipped by Bubblewrap; existing sources keep
their documented permissions.

## Configuration

The optional settings file is `$XDG_CONFIG_HOME/bwrap-wrapper/config.toml` when
`XDG_CONFIG_HOME` is set and nonempty, or `$HOME/.config/bwrap-wrapper/config.toml`
otherwise. Relative configuration homes are made absolute from the working
directory, retaining `..` without resolving symlinks, just like mount discovery.
The file sits alongside the existing `bwrap-wrapper/cwd/` overlay directory;
it is not mounted into the workspace.

Invocation logging is **disabled by default** for both Codex and other calls.
Enable it explicitly with:

```toml
[logging]
enabled = true
```

Disable it with:

```toml
[logging]
enabled = false
```

A missing or empty file, missing `[logging]` section, or missing `enabled` key
also disables logging. `logging.enabled` must be a boolean; strings such as
`"true"` and numbers are invalid. Unsupported keys are ignored.

Settings are read and validated before preparing any invocation log. An existing
file that cannot be read, invalid TOML, or an incorrect type for a supported
setting produces a stderr diagnostic containing the configuration path and exits
nonzero, without creating log artifacts or executing Bubblewrap.

`BWRAP_LOG` only selects the destination when `logging.enabled = true`; setting
it never enables logging. With logging disabled, no log files or parent
directories are created, existing logs remain untouched, and empty or unusable
`BWRAP_LOG` values do not prevent execution. Timestamp and username lookup and
environment collection for logging are skipped. Wrapper errors still go to
stderr, and the executed program retains its stdout and stderr.

## Codex invocation detection and workspace

Two invocation formats are supported:

- Legacy: an adjacent `--argv0 codex-linux-sandbox` pair before the first `--`,
  then an executable path containing `/` with basename exactly `codex`.
- Current: immediately after the first `--`, an executable with exact basename
  `codex-linux-sandbox` (including a bare helper name, without a trailing slash).

Other calls are forwarded unchanged and logged only when enabled. The first
workspace option, `--sandbox-policy-cwd PATH` or `--sandbox-policy-cwd=PATH`, supplies
the workspace.
Legacy calls scan the entire argument list as Python does. Current calls scan
only the helper arguments before its nested `--`, excluding shell arguments.
A missing final value or an empty value means unspecified.
An option followed by another token consumes that token as its value, including
`--`. Nonempty values are normalized lexically with Python `os.path.normpath`
semantics and must be absolute. Symlinks are not resolved and existence is not
required. Exactly two leading slashes are preserved; three or more become one.
The original policy arguments remain forwarded as supplied.

## Mount modifications

Only recognized Codex calls receive these changes before the command separator:

- `--dev /dev` becomes `--dev-bind /dev /dev`. Other device arguments stay intact.
- For each target path `P`, remove every exact protection group. Targets are
  `WORKSPACE/.git`, `WORKSPACE/.agents`, `WORKSPACE/.codex`, and `WORKSPACE/.aws`
  when a workspace is specified, plus the fixed absolute paths `/tmp/.git`,
  `/tmp/.agents`, `/tmp/.codex`, and `/tmp/.aws`:

  ```text
  --ro-bind P P
  --perms MODE --tmpfs P --remount-ro P
  --tmpfs P --remount-ro P
  ```

  Both bind operands must equal `P`. A tmpfs mask is removed only together with
  its adjacent matching read-only remount, including the associated `--perms MODE`
  prefix when present. Incomplete groups are retained; matching never crosses
  the command separator. Other permission modifiers and mounts stay intact.
  Workspace targets use the normalized workspace, but mount operands are compared
  byte for byte, without normalization or symlink resolution. The fixed `/tmp`
  targets apply regardless of workspace location, including when no workspace is
  specified; an invalid workspace still fails. `/tmp` is literal, independent of
  `$TMPDIR`. When the workspace is `/tmp`, the overlapping targets use the same
  rules. Other paths, nested paths such as `/tmp/subdir/.aws`, and near matches
  such as `/tmp/.aws-backup` or spellings with redundant dots or slashes remain.
  The workspace or `/tmp` mount supplies access to these paths. In particular,
  `--bind /tmp /tmp`, daemon masks such as
  `--perms 000 --tmpfs /tmp/codex-daemon-1000 --remount-ro /tmp/codex-daemon-1000`,
  and binds of `/tmp/codex-bwrap-synthetic-mount-targets-1000` remain intact,
  regardless of their suffixes. This changes only forwarded arguments; no
  directories or configuration files are deleted or modified on disk.
  The expanded targets and tmpfs removal are an intentional
  [change from the Python reference](compatibility.md#project-local-mount-protections).
- Before the first exact `--bind WORKSPACE WORKSPACE`, insert this sequence:

  ```text
  --tmpfs HOME
  --dir HOME/.codex
  --ro-bind-try HOME/.codex/packages HOME/.codex/packages
  --bind-try HOME/.codex/memories HOME/.codex/memories
  --ro-bind-try HOME/.codex/AGENTS.md HOME/.codex/AGENTS.md
  ```

  This happens once. Each original workspace bind remains. Without a matching
  bind, these HOME mounts and configuration overlays are not inserted.
- After **each** matching workspace bind, discover configuration files anew and
  insert a read-only bind for each direct regular file in
  `$XDG_CONFIG_HOME/bwrap-wrapper/cwd`, targeting `WORKSPACE/<filename>`.
  Hidden files and symlinks to regular files are included. Subdirectories,
  symlinks to directories, broken links, and other non-file entries are skipped.
  Names are sorted using Python's filesystem-name ordering. Missing, empty, or
  non-directory configuration paths add no mounts. Non-missing I/O errors abort.
  Relative configuration homes are made absolute from the current directory;
  `..` is retained and symlinks are not resolved.
- Immediately before the separator, read `/proc/mounts` and insert
  `--tmpfs MOUNTPOINT` for each Windows drive detected as filesystem `9p` with
  the exact option `aname=drvfs` among comma/semicolon-separated options. Decode
  three-digit octal path escapes, remove duplicates, and sort by slash count then
  name, so nested mounts follow their ancestors. A mount-table read or UTF-8
  decoding failure aborts. An ordinary Linux machine may have no such mounts.

These changes expose the host's `/dev`, remove the workspace and fixed `/tmp`
mount protections above, hide HOME behind tmpfs with selected Codex paths
re-exposed, make Codex memories writable, and overlay detected Windows-drive
mounts. They preserve the remaining mount policy, with optional missing sources
tolerated. No additional filtering or mount validation is applied.

The rewrite loop retains the reference's malformed-option behavior. For example,
`--bind SOURCE --` consumes the separator as a destination; a consumed separator
is not a point at which mount discovery runs. Otherwise the separator and all
command arguments after it are preserved byte for byte.

## Logging and failures

At startup the process sets umask `0077` and validates settings. When logging is
enabled, it then creates the log's parent directories and captures local timestamp
and username for the current UID before workspace validation. `BWRAP_LOG` overrides
the default `$HOME/.local/state/bwrap-wrapper.log` destination. New log files
normally get mode `0600`, and new directories `0700`.
Existing log permissions are preserved. Entries are appended between
`========== BWRAP START ==========` and `=========== BWRAP END ===========`:

```text
Timestamp: 2026-11-04T21:58:33+09:00
User: user
PID: 12345
Codex invocation: True
Bwrap: /usr/bin/bwrap
Sandbox CWD: /workspace
Tmpfs inserted: True
Original args: ...
Modified args: ...

Environment:
NAME=value
```

Booleans use Python's `True`/`False` spelling. Arguments use POSIX shell quoting
matching `shlex.quote`. The entire environment is included in name order. Logs
may contain credentials and other secrets: protect them and redact before
sharing. There is no rotation or automatic redaction.

Enabled logging uses strict UTF-8 like Python's `open(..., encoding="utf-8")`.
Although parsing and rewriting preserve Unix argument bytes, invalid UTF-8 in a logged
argument, path, username, or environment entry prevents execution. This preserves
Python's failure on surrogateescaped data. A failed attempt may leave a partial
log entry; log-directory creation can occur even when workspace validation fails.
With logging disabled, Unix argument and environment bytes are forwarded without
this logging-only UTF-8 validation.

Missing HOME, invalid settings or workspace, discovery failures, enabled logging
failures, and failed exec all stop with a nonzero status. There is no fallback to
unmodified arguments.
See the [audit](compatibility.md) for validation boundaries and diagnostic details.

See [real mount isolation results](isolation.md) for verified permissions and
the narrow read-only mount for generated helpers under `.codex/tmp/arg0-linux`.

Optional HOME mounts require Bubblewrap with `--bind-try` and `--ro-bind-try`
support (confirmed in the installed 0.12.0 `--help`). No older version range
has been validated. Check the real binary's `--help` when deploying elsewhere.
Only missing sources are skipped; other mount errors still fail startup.
