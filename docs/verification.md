# Verification

## Smoke test with a fake executable

Run this from the project root. It installs both variants into a temporary
directory and uses a fake `bwrap` that prints one bracketed line per argument.

```sh
(
    set -eu
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    mkdir -p "$work/real" "$work/home" "$work/source/bwrap_wrapper"
    install -m 755 bwrap "$work/source/bwrap"
    install -m 644 bwrap_wrapper/*.py "$work/source/bwrap_wrapper/"
    /usr/bin/python3.9 build_zipapp.py --output-dir "$work/archive"
    cat > "$work/real/bwrap" <<'FAKE'
#!/bin/sh
printf '<%s>\n' "$@"
FAKE
    chmod 755 "$work/real/bwrap"
    for launcher in "$work/source/bwrap" "$work/archive/bwrap"; do
        HOME="$work/home" PATH="$work/source:$work/archive:$work/real" \
            "$launcher" --dev /dev 'two words' ''
    done
)
```

Each run prints these same four lines. The arguments have no Codex command
separator, so they pass through as supplied.

```text
<--dev>
</dev>
<two words>
<>
```

## Ordinary tests

```sh
python3.9 -m unittest -v
python3.14 -m unittest -v
```

These tests use fake executables, synthetic files, and injected mount readers.
No namespace support is required. The Rust comparisons and real isolation
checks are opt-in.

## Rust differential tests

Use the nested `rust/` checkout on `main` at `1446c5f` and a Rust 1.85 or
newer toolchain. Build into a separate target directory to keep that checkout
clean.

```sh
(
    set -eu
    target=$(mktemp -d)
    trap 'rm -rf "$target"' EXIT
    cargo build --locked --manifest-path rust/Cargo.toml \
        --target-dir "$target" --lib --bin bwrap
    rustc --edition=2024 tests/rust_mount_driver.rs \
        --extern bwrap_wrapper_rs="$target/debug/libbwrap_wrapper_rs.rlib" \
        -L "dependency=$target/debug/deps" -o "$target/mount-driver"
    export BWRAP_RUST_BIN="$target/debug/bwrap" BWRAP_RUST_DRIVER="$target/mount-driver"
    python3.9 -m unittest -v tests.test_differential
    python3.14 -m unittest -v tests.test_differential
)
```

The shared fixtures use equivalent JSON and TOML settings and the same
synthetic mount table and fake executable. The tests compare forwarded
bytes, exit or signal outcomes, and complete successful logs with only the
timestamp and PID normalized. Failure cases check diagnostic categories
and which log artifacts were created.

## Real mount isolation

Run with real Bubblewrap executables that are usable on the host. The
legacy format needs a build accepting `--argv0`. For example, Bubblewrap
0.6.3 can exercise the current format and a Codex-bundled Bubblewrap build
with `--argv0` can exercise the legacy format.

```sh
BWRAP_ISOLATION_CURRENT=/usr/bin/bwrap \
    python3.9 -m unittest -v tests.test_isolation

BWRAP_ISOLATION_LEGACY=/absolute/path/to/compatible/bwrap \
    python3.9 -m unittest -v tests.test_isolation
```

Set both variables to run both formats together. Each format runs against
both launcher variants and workspaces inside and outside HOME, with all
eight combinations of packages, memories, and `AGENTS.md` present or absent.
That is 32 cases per format. Fixtures are synthetic, with a read-only root
mount and writable binds into temporary directories. The checks cover HOME
hiding, generated helper execution and directory permissions, optional
sources, workspace writes, overlays, configured extra directories, and
unrelated read-only mounts. Real WSL2 drive masking and end-to-end Codex
sessions require target-host checks in addition to this suite.
