# Safe smoke test

This procedure uses a temporary shell script as Bubblewrap. It performs no
mounts and needs no namespace privileges. It exercises the actual Rust executable
with a non-Codex call, so it also avoids depending on host drive mounts.

Run from the repository root:

```sh
cargo build --release
smoke_dir=$(mktemp -d)
cat > "$smoke_dir/bwrap" <<'SH'
#!/bin/sh
printf 'replacement PID: %s\n' "$$"
printf 'argument: <%s>\n' "$@"
printf 'inherited marker: %s\n' "$SMOKE_MARKER"
exit 23
SH
chmod 755 "$smoke_dir/bwrap"
env -i PATH="$smoke_dir:/usr/bin:/bin" HOME="$smoke_dir/home" \
  BWRAP_LOG="$smoke_dir/invocations.log" SMOKE_MARKER=visible \
  ./target/release/bwrap --dev /dev -- /bin/true 'two words'
smoke_status=$?
printf 'exit status: %s (expected 23)\n' "$smoke_status"
cat "$smoke_dir/invocations.log"
rm -r -- "$smoke_dir"
```

Use a shell without `set -e`, or guard the invocation so the expected status 23
does not stop your shell before the status capture. Expect unchanged arguments,
`inherited marker: visible`, `Codex invocation: False`, and status 23. This
procedure clears the inherited environment before invocation; ordinary wrapper
logs can contain secrets and should be protected.
The temporary variable is deliberately named `smoke_dir` and HOME is changed only
for that one command.

For Codex mount rewrites with synthetic drive mounts, use `cargo test` and the
optional direct reference check described in [compatibility.md](compatibility.md).
Those use injected fixture data, never a real Bubblewrap process.
