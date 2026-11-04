# bwrap-wrapper-rs

A Linux Rust port of `bwrap-wrapper`, compatible with the Python reference at
revision `e614a089b80b6f7832edd66950bba8f06abe0040`. It targets stable Rust
2024, uses the small `libc` crate for POSIX process and time calls, and does not
need Python at runtime.

Build and verify it with:

```sh
cargo build --release
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The Cargo package is `bwrap-wrapper-rs`, and the executable is `bwrap`. Put it
earlier in `PATH` than Codex's Bubblewrap lookup. Set `BWRAP_REAL` to the actual
Bubblewrap path if needed, so the shim does not call itself recursively.

## Behavior

Every invocation requires `HOME`, logs the original and forwarded arguments,
then replaces itself with Bubblewrap via `execv`. Non-Codex calls preserve their
arguments. A Codex invocation must have `--argv0 codex-linux-sandbox` before the
first `--` and a command path ending in `/codex` immediately after it. For those
calls the wrapper performs the Python shim's workspace normalization, `/dev`
rewrite, exact `.git` overlay removal, HOME mounts, configuration-file overlays,
and WSL2 drvfs masking.

`HOME` is required, including for non-Codex calls. `BWRAP_REAL` defaults to
`/usr/bin/bwrap`; `BWRAP_LOG` defaults to `$HOME/.local/state/bwrap-wrapper.log`;
and `XDG_CONFIG_HOME` defaults to `$HOME/.config` when unset or empty. Empty
`BWRAP_REAL` and `BWRAP_LOG` values remain empty, matching the reference.

For Codex calls, `/proc/mounts` is read and parsed for `9p` mounts with the exact
`aname=drvfs` option. Matching mount points are deduplicated, octal escapes are
decoded, and tmpfs mounts are inserted before `--`. The configured
`bwrap-wrapper/cwd` directory contributes its direct regular files, including
hidden files and symlinks to regular files, in Python filename order. Directory
entries are read once for the invocation and their mount arguments are inserted
after each matching workspace bind.

The log records a local ISO 8601 timestamp, user, PID, Codex detection, Bubblewrap
path, workspace, HOME mount status, quoted arguments, and every environment
variable in name order. Existing log permissions are retained; new files use the
startup umask `0077`. Logs can contain secrets from arguments and environment
values. Protect them and redact before sharing.

## Implementation boundary

The argument transformer takes explicit inputs and stays independent of
filesystem discovery and process replacement. Mount parsing is separately
testable with synthetic data. Integration tests use temporary fake executables;
they do not run Bubblewrap or require mount privileges. WSL2 mount visibility
has not been tested on a WSL2 host.
