# Usage and behavior

## Build and installation

Supported targets are Linux and WSL2 Linux distributions. Use a stable Rust
compiler supporting edition 2024 (1.85+), Cargo, and a system linker. The only
Cargo dependency is `libc`. Actual sandboxing needs a working Bubblewrap and the
host's normal user/mount namespace support. Tests use fake executables instead.

From the repository root:

```sh
cargo build --release
test -x /usr/bin/bwrap
install -Dm755 target/release/bwrap "$HOME/.local/lib/bwrap-wrapper/bin/bwrap"
export BWRAP_REAL=/usr/bin/bwrap
export PATH="$HOME/.local/lib/bwrap-wrapper/bin:$PATH"
codex
```

Adjust `/usr/bin/bwrap` to the real installed Bubblewrap path before changing
PATH. The wrapper binary is `target/release/bwrap`; the Cargo package is
`bwrap-wrapper-rs`. Do not overwrite the real Bubblewrap. `command -v bwrap`
after the PATH change should identify the wrapper, so do not use that result as
`BWRAP_REAL`. Persist the exports in your shell configuration if desired.

`BWRAP_REAL` is passed to `execv` without searching PATH. Absolute paths are
recommended; a relative value is interpreted from the process working directory.
The wrapper preserves PID, environment, standard streams, exit status, and
signal termination by replacing the process. Execution failure exits nonzero.

## Environment

| Variable | Unset default | Empty value |
| --- | --- | --- |
| `HOME` | Required; absence fails even for non-Codex calls | Accepted, as in Python |
| `BWRAP_REAL` | `/usr/bin/bwrap` | Preserved; exec fails |
| `BWRAP_LOG` | `$HOME/.local/state/bwrap-wrapper.log` | Preserved; logging fails |
| `XDG_CONFIG_HOME` | `$HOME/.config` | Uses `$HOME/.config` |

There is no separate settings file. No missing mount source is automatically
created. When HOME mounts are inserted, the packages, memories, and AGENTS.md
sources described below must exist for real Bubblewrap to mount them.

## Codex invocation detection and workspace

Both conditions are required:

1. Before the first `--`, an adjacent pair is exactly
   `--argv0 codex-linux-sandbox`.
2. The next argument after that first separator contains `/` and has basename
   exactly `codex`. `/opt/codex` and `./codex` qualify; `codex` and `/opt/codex/`
   do not.

Other calls are logged and forwarded unchanged. For a recognized call, the first
`--sandbox-policy-cwd PATH` or `--sandbox-policy-cwd=PATH` anywhere in the original
argument list supplies the workspace. This scan includes command arguments after
`--`, as in Python. A missing final value or an empty value means unspecified.
An option followed by another token consumes that token as its value, including
`--`. Nonempty values are normalized lexically with Python `os.path.normpath`
semantics and must be absolute. Symlinks are not resolved and existence is not
required. Exactly two leading slashes are preserved; three or more become one.
The original policy arguments remain forwarded as supplied.

## Mount modifications

Only recognized Codex calls receive these changes before the command separator:

- `--dev /dev` becomes `--dev-bind /dev /dev`. Other device arguments stay intact.
- Every exact `--ro-bind WORKSPACE/.git WORKSPACE/.git` is removed. Other mounts,
  including spellings with redundant dots or slashes, stay intact. Comparison is
  against the normalized workspace string, without normalizing bind operands.
- Before the first exact `--bind WORKSPACE WORKSPACE`, insert this sequence:

  ```text
  --tmpfs HOME
  --dir HOME/.codex
  --ro-bind HOME/.codex/packages HOME/.codex/packages
  --bind HOME/.codex/memories HOME/.codex/memories
  --ro-bind HOME/.codex/AGENTS.md HOME/.codex/AGENTS.md
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

These changes expose the host's `/dev`, remove the workspace `.git` read-only
overlay, hide HOME behind tmpfs with selected Codex paths re-exposed, make Codex
memories writable, and overlay detected Windows-drive mounts. They reproduce the
reference's policy. No additional filtering or mount validation is applied.

The rewrite loop retains the reference's malformed-option behavior. For example,
`--bind SOURCE --` consumes the separator as a destination; a consumed separator
is not a point at which mount discovery runs. Otherwise the separator and all
command arguments after it are preserved byte for byte.

## Logging and failures

At startup the process sets umask `0077`, creates the log's parent directories,
and captures local timestamp and username for the current UID before workspace
validation. New log files normally get mode `0600`, and new directories `0700`.
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

Logging uses strict UTF-8 like Python's `open(..., encoding="utf-8")`. Although
parsing and rewriting preserve Unix argument bytes, invalid UTF-8 in a logged
argument, path, username, or environment entry prevents execution. This preserves
Python's failure on surrogateescaped data. A failed attempt may leave a partial
log entry; log-directory creation can occur even when workspace validation fails.

Missing HOME, invalid workspace, discovery failures, logging failures, and failed
exec all stop with a nonzero status. There is no fallback to unmodified arguments.
See the [audit](compatibility.md) for validation boundaries and diagnostic details.
