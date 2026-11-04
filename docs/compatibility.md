# Compatibility audit and verification

The source audit compares every branch of the supplied Python `bwrap_shim.py`
and its README at revision `e614a089b80b6f7832edd66950bba8f06abe0040` against the
Rust rewrite. The original Apache-2.0 license is preserved verbatim in `LICENSE`.
There is no original NOTICE file in the supplied reference. Rust source is an
explicit rewrite, with the reference revision acknowledged in the crate and README.

## Audit results

| Behavior | Source comparison and regression evidence |
| --- | --- |
| Codex detection | Exact adjacent argv0 pair before first separator, then a path containing `/` with basename `codex`; false-positive and trailing-slash cases |
| Workspace | First option in either format across the entire argument list, empty/missing handling, relative rejection, lexical dots/parents/slashes, double leading slash |
| Exact bind matching | Original operands are not normalized; repeated workspace binds preserved; unrelated `.git` mounts retained |
| HOME sequence | Exact ordered sequence once, before first matching bind; missing sources never created |
| Configuration | Discovery after every matching bind; hidden files and file symlinks; no recursion; missing/empty directories; deterministic name order; metadata errors propagated |
| Windows drives | Synthetic 9p tables, comma/semicolon options, exact token matching, octal decoding, duplicates, nested paths, Python whitespace, CR/LF input |
| Command tail | First unconsumed separator and remaining arguments retained; malformed bind consuming separator does not trigger mounts |
| Non-Codex calls | No workspace validation or discovery, arguments unchanged, logging still required |
| Environment | Required HOME; unset and empty variable semantics; relative XDG paths; empty/relative executable paths |
| Logs | Python boolean spelling, local ISO timestamp with offset seconds when needed, UID username, PID, shlex quoting, sorted environment, permissions, failures before exec |
| Unix bytes | Pure transformation preserves invalid UTF-8; runtime logging rejects it like Python's strict encoder, including invalid environment values |
| Process execution | Actual execv, no PATH search, same PID, inherited environment and stdin/stdout/stderr, exit code and signal propagation; no execution on error |

The Phase 3 audit corrected these differences from the previous Rust phases:

- `%:z` is not supported by this host's libc strftime and was appearing literally
  in timestamps. Offsets are now formatted from the local `tm_gmtoff` value.
- Rust boolean strings were lowercase; log fields now use Python spelling.
- Configuration files were read once. Discovery now runs at each matching bind,
  so later binds can reflect changes or errors that arise between them.
- Log-directory and identity preparation now precede workspace parsing.
- The previous runtime accepted invalid UTF-8 log data and executed anyway.
  Strict logging now fails before exec, matching Python.
- Non-missing metadata errors were silently treated as absent configuration;
  these now propagate as pathlib does in the audited CPython 3.13.5 environment.
- Absolute configuration homes no longer require reading the current directory,
  including when the process's working directory has been deleted.
- Mount parsing includes Python's additional whitespace characters and universal
  newline behavior. Discovery callbacks replace duplicated runtime argument scans.
- Buffered log writes and explicit close-error handling complete before exec.

## Current sandbox helper compatibility extension

In addition to the legacy `--argv0 codex-linux-sandbox -- /path/to/codex`
format, Rust recognizes `-- /path/to/codex-linux-sandbox` (also a bare helper
name). The executable must occur immediately after Bubblewrap's first `--`,
with the exact basename and no trailing slash. Similar names and helper names
inside shell arguments do not qualify.

For this newer format, workspace options (`--sandbox-policy-cwd PATH` or
`--sandbox-policy-cwd=PATH`) are read only from the helper's arguments, stopping
at its nested `--`. Empty or missing values yield no workspace; relative values
are rejected and absolute paths retain the existing lexical normalization.
Legacy calls retain their original whole-list workspace scan. Both formats use
the same mount transformations and preserve the first separator and entire
command tail verbatim. Logging uses the same recognition and insertion flags.

This is an intentional Rust compatibility extension; the Python reference is
unchanged. `tests/fixtures/current-sandbox.txt` is a sanitized argument fixture,
with no captured environment or raw invocation log. Automated tests cover its
mounts, overlays, command tail, and runtime log fields using fake execution.

## Test commands

Normal builds and tests need no Python, installed Bubblewrap, host WSL mounts, or
mount privileges. Install your Rust toolchain's rustfmt and Clippy components if
needed, then run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

`tests/compatibility.rs` tests transformation with explicit fixture inputs and
filesystem discovery with temporary files. `tests/runtime.rs` runs the actual
binary against temporary fake executables for passthrough and process behavior.
For recognized Codex calls, a child test driver runs the same runtime with an
injected mount reader and synthetic mount files. This introduces no production
environment-variable override and no dependency on the host's `/proc/mounts`.
The driver is inert during an ordinary test run.

For a direct comparison with the original Python implementation, leave the
reference checkout at `./python` and run this optional development check:

```sh
python3 scripts/check_reference.py
```

The script builds the Rust test driver, imports the reference with a mock mount
reader, and runs both against the same temporary fake executable. It compares
success/failure, exact forwarded argument bytes, and complete successful log
entries except for naturally different timestamps and PIDs. Cases cover Unicode,
quotes, malformed and missing options, multiple binds and Git overlays, synthetic
nested/escaped/duplicate mount points, hidden files, file/directory/broken
symlinks, missing/empty/relative configuration homes, redundant path separators,
empty HOME, invalid UTF-8 arguments, and execution/logging failures. Randomized
cases use a fixed seed. Only fixture environment variables are logged.

## Results in this checkout

Verified on Linux with rustc/Cargo 1.85.1 and CPython 3.13.5:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 38 tests passed (including the inert child-fixture entry point).
- `cargo build --release`: passed.
- `python3 scripts/check_reference.py`: 332 deterministic cases matched for
  outcome, forwarded bytes, and complete successful logs apart from time/PID.
- The documented fake-executable smoke procedure passed with unchanged arguments,
  an inherited marker, exit status 23, and a complete log.

The environment's missing rustfmt and Clippy commands were supplied from
matching Debian Rust 1.85.1 packages extracted temporarily under `/tmp`; no
project or system toolchain change is required by the implementation.

## Remaining differences and limits

- Rust reports concise errors instead of CPython exception tracebacks. Both fail
  nonzero without executing Bubblewrap. Reproducing Python source stack frames
  would tie diagnostics to a Python interpreter that is intentionally absent at
  runtime; this does not affect successful Codex calls or mount policy.
- The audit used CPython 3.13.5 in UTF-8 mode. Rust mirrors its pathlib policy:
  ENOENT, ENOTDIR, EBADF, and ELOOP mean no matching path; other metadata errors
  abort. The behavior of other Python versions has not been validated. This
  keeps non-missing filesystem errors fail-closed rather than silently skipping
  a mount source.
- Failed logs may differ in partial bytes already written and diagnostic wording.
  Complete successful logs are compared directly. Concurrent invocations can
  interleave appends, and neither implementation provides log rotation, locking,
  or crash-durable logging.
- Locale comparisons assume the usual Linux UTF-8 filesystem decoding used by
  the audited Python process. Rust always applies strict UTF-8 to log fields.
- Real Bubblewrap namespace permissions, source mount availability, host `/dev`
  access, effective workspace overlays, and Windows-drive visibility still need
  manual validation on the target Linux/WSL2 host. Synthetic parsing and fake
  execution demonstrate argument construction and process behavior, not actual
  mount isolation. Ordinary tests do not run real Bubblewrap. The separate opt-in
  [isolation suite](isolation.md) has verified synthetic mounts on this Linux
  host and records a helper startup incompatibility.

## Project structure

```text
Cargo.toml / Cargo.lock      package, executable, and libc dependency
LICENSE                     original Apache-2.0 license
README.md                   essential setup and links
src/main.rs                 executable entry and nonzero errors
src/runtime.rs              environment, discovery orchestration, execv
src/transform.rs            argument parsing and rewrite with injectable discovery
src/config.rs               configuration paths, sorting, and regular-file discovery
src/mounts.rs               pure mount parser and actual mount-table reader
src/logging.rs              strict UTF-8 logging, quoting, time, and UID identity
src/paths.rs                Python-compatible Unix path operations
src/lib.rs                  reusable core and integration modules
tests/compatibility.rs      parser and transformation regressions
tests/runtime.rs            fake-executable and deterministic mount integration
scripts/check_reference.py  optional Python reference comparison
docs/usage.md               full usage and behavior
docs/smoke-test.md          safe fake-executable smoke procedure
docs/compatibility.md      audit, regression approach, and limitations
```

The supplied `python/` checkout is a development reference and remains untracked;
it is not required to build or run the Rust executable.
