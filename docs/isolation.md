# Real Bubblewrap isolation verification

Run separately from the ordinary unit and fake-executable tests:

```sh
cargo test --test isolation -- --ignored --nocapture
# If installed elsewhere, specify the absolute real binary, never the wrapper:
BWRAP_ISOLATION_REAL=/usr/bin/bwrap cargo test --test isolation -- --ignored --nocapture
```

The test requires Linux mount namespaces and a working Bubblewrap installation.
Missing binaries or insufficient privileges fail the test rather than silently
substituting fake execution. The test is ignored during ordinary `cargo test`.
All HOME, workspace, helper, and configuration content is synthetic and created
under a temporary directory; the child receives a cleared environment. No real
Codex profile or credentials are used. The sandbox root is read-only; writable
binds point only at fixture directories. Fixture files are removed afterwards.

## Results

On this checkout's Linux host, `/usr/bin/bwrap` executed successfully. The
ignored integration test passed all four combinations: legacy/current helper
format and workspace outside/inside HOME. Each invocation begins with
`--ro-bind / /` and tests effective access from inside the mount namespace:

- HOME is tmpfs (checked using `stat -f`); an original private file is invisible.
- Packages and AGENTS.md can be read, but writes fail.
- Memories can be read and updated; the update reaches the fixture on the host.
- Workspace creation succeeds and persists on the host, including under HOME.
- `.git/config` is readable and writable after removal of the exact read-only
  overlay. `.git` is neither hidden nor denied.
- An unrelated read-only directory mount remains readable and rejects writes.
- A configuration overlay supplies its replacement contents and rejects writes.
- A synthetic helper in the allowed packages directory starts and executes the
  nested shell command successfully.
- Loading a shell snapshot from HOME fails because the snapshot is hidden.
- Executing a helper at `.codex/tmp/arg0-linux/example/codex-linux-sandbox`
  fails with `No such file or directory` after HOME isolation.

The Python branch's `bwrap_shim.py` was inspected directly: its HOME sequence is
tmpfs, `.codex` directory, packages read-only, memories writable, AGENTS.md
read-only, followed by the original workspace bind. The Rust mount order and
allowlist match this policy. No mount policy change was made.

## Confirmed runtime compatibility limitation

The observed new invocation launches its helper from `.codex/tmp`. That helper
is hidden by the existing HOME tmpfs, so recognizing the invocation and inserting
the reference mounts can prevent startup. Shell snapshots outside the allowed
paths are also hidden; a command that requires one fails. These failures are
reproduced with synthetic files and real Bubblewrap, not an installed Codex
session. Full Codex startup and execution remain unverified.

Exposing `.codex/tmp` or shell snapshots would expand access to host runtime
files. Binding the whole directories could expose unrelated sessions' artifacts;
snapshots may contain environment data. A future change should establish which
specific files are required, choose the smallest mounts and permissions, and
review their lifetimes before expanding access. This verification deliberately
retains the requested Python policy and records the incompatibility instead of
silently widening it.

Later explicit binds of HOME or a HOME ancestor can reveal its contents again;
the wrapper preserves original binds and does not enforce an allowlist over
arbitrary later mounts. The tested invocation has no such later broad bind.
The tests establish the documented fixture's behavior, not isolation for every
possible Bubblewrap argument sequence. Real WSL drive masking and an actual
Codex session still require target-host validation.
