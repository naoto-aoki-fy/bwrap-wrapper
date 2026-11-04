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

## Optional-source regression matrix

The suite now runs all eight presence combinations of packages, memories, and
AGENTS.md across both invocation formats and both workspace locations (32 cases).
When packages are absent, the executable fixture lives outside HOME so absent
packages cannot hide it.
It checks skipped paths, host non-creation, existing contents and permissions,
and the isolation checks below. Bubblewrap 0.12.0 advertises both try options.
On the current restricted runner, the real suite is blocked by
`Creating new namespace failed: Operation not permitted`; the expanded matrix
requires a namespace-capable host. The earlier results below predate this change.

## Generated-helper validation on this runner

Ordinary tests pass (45 tests; the real isolation test is ignored by default).
Formatting and Clippy pass using the matching tools under /tmp; Clippy requires
`-A unknown-lints` for an existing allowance unsupported by this toolchain.
The real isolation suite was attempted and stopped at its first case with
`Creating new namespace failed: Operation not permitted`. Consequently the
updated symlink execution and read-only isolation checks require a
namespace-capable host; they have not been verified here.

## Previous results

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
## Generated helper directories

For a current helper command exactly at
`HOME/.codex/tmp/arg0-linux/<directory>/codex-linux-sandbox`, HOME isolation
adds this sequence after the existing packages, memories, and AGENTS.md mounts
and before the workspace bind:

```text
--dir HOME/.codex/tmp
--dir HOME/.codex/tmp/arg0-linux
--ro-bind HOME/.codex/tmp/arg0-linux/<directory> HOME/.codex/tmp/arg0-linux/<directory>
```

The parent directories are empty sandbox directories. Only the immediate helper
directory is exposed, read-only, including any siblings inside that directory.
Other generated directories and unrelated tmp files remain hidden. No host
files are created or changed. The command path is preserved, so a helper symlink
can resolve into the existing read-only packages mount.

The matcher rejects dot components, traversal, extra nesting, empty directory
names, and paths outside this exact HOME prefix. It does not resolve the helper
symlink. Missing sources fail at the mandatory read-only bind; missing executables
or inaccessible symlink targets fail at execution, without a broader fallback.
As with existing mounts, source directories are trusted host paths; concurrent
host replacement and symlinked source ancestors are not prevented by lexical
validation. Legacy calls and calls without HOME isolation receive no helper mount.

The integration matrix now executes a synthetic symlink from a generated helper
directory whenever packages are present in the current invocation format. It
checks that the directory rejects writes and unrelated tmp content remains
hidden. Cases with absent packages retain an external helper fixture.

Shell snapshots remain hidden. Full installed Codex execution still requires
target-host validation.

Later explicit binds of HOME or a HOME ancestor can reveal its contents again;
the wrapper preserves original binds and does not enforce an allowlist over
arbitrary later mounts. The tested invocation has no such later broad bind.
The tests establish the documented fixture's behavior, not isolation for every
possible Bubblewrap argument sequence. Real WSL drive masking and an actual
Codex session still require target-host validation.
