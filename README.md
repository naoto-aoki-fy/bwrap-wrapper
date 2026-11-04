# bwrap-wrapper-rs

Phase 1 compatibility port of `bwrap-wrapper`, based on the supplied Python
checkout at revision `e614a089b80b6f7832edd66950bba8f06abe0040`.
The reference source and README were inspected directly. This is a Rust rewrite
of that Apache-2.0 project; the original license is retained in `LICENSE`.

Requires Linux (including WSL2) and stable Rust 1.85 or newer, edition 2024.
There are no external Cargo dependencies and no Python runtime dependency.

```sh
cargo build
cargo fmt --check
cargo test
```

The Cargo package is `bwrap-wrapper-rs`; its executable is `bwrap`.
**Do not install this Phase 1 executable into PATH as a Bubblewrap replacement.**
It currently prints a phase-status message and exits with status 1. Logging,
environment acquisition, `/proc/mounts` reading, umask changes, and execution of
real Bubblewrap are deferred to Phase 2. No tests execute Bubblewrap.

## Core API

- `transform::is_codex` requires `--argv0 codex-linux-sandbox` before the first
  separator and an immediately following command path ending in `/codex`.
- `transform::workspace` scans all arguments for the first policy option,
  including arguments after `--`. Empty or absent values are unspecified;
  nonempty values use Python lexical `normpath` semantics and must be absolute.
  The original policy arguments remain in the output.
- `transform::transform` accepts original `OsString` arguments and explicit
  `Inputs`: HOME, discovered configuration files, and Windows mount points.
  Non-Codex arguments pass through unchanged without workspace validation.
  Recognized calls rewrite `--dev /dev`, remove exact workspace `.git` overlays,
  inject HOME mounts before the first exact workspace bind, and insert file
  overlays after every matching bind. Windows tmpfs mounts come immediately
  before the command separator. Command arguments stay unchanged.
- `config::directory` takes HOME, optional XDG_CONFIG_HOME, and the absolute
  process working directory explicitly. It follows Python `Path.absolute()`:
  relative configuration homes become absolute without resolving `..` or
  symlinks. An unset or empty XDG value falls back to `$HOME/.config`.
- `config::files` reads direct regular files, including hidden files and symlinks
  to regular files, in Python filename order. It excludes directories, broken
  symlinks, and nested files. Missing or non-directory paths yield no overlays;
  directory listing errors propagate. It never creates mount sources.
- `mounts::parse` accepts UTF-8 mount-table text, selects `9p` with the exact
  comma/semicolon-separated option `aname=drvfs`, decodes three-digit octal
  escapes, deduplicates, and sorts by slash count then name.

Supply configuration files in discovery order and mount points in parser order.
Discovery is intentionally separate: a future caller should discover mounts
only for recognized invocations, and configuration only when a workspace bind
matches. Phase 1 snapshots explicit discovery results; the Python program
re-reads configuration after each bind, so concurrent filesystem changes between
repeated binds are outside this pure transformation's model.

## Compatibility boundaries

Argument paths use Unix bytes without lossy UTF-8 conversion. Exact mount
comparisons retain redundant slashes and dots in original arguments; only the
workspace-policy value is normalized. Exactly two leading slashes are retained
by Python-style normalization. Configuration sorting also reproduces Python's
UTF-8/surrogateescape order for non-UTF-8 filenames. Octal mount escapes follow
Python `chr()` semantics, including escapes above ASCII.

The scan retains the reference's handling of malformed options, including
consumption of `--` as a bind operand. No extra argument validation is applied.
Tests cover synthetic Linux/WSL2 mount data and temporary configuration trees;
actual WSL2 mount isolation and runtime integration are not validated in Phase 1.
