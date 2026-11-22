# Verification

## Smoke test with a fake executable

Run this from the project root. It installs the wrapper into a temporary uv
tool environment and runs it outside the checkout with a fake `bwrap` that
prints one bracketed line per argument.

```sh
(
    set -eu
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    mkdir -p "$work/real" "$work/home"
    export UV_TOOL_DIR="$work/tools" UV_TOOL_BIN_DIR="$work/bin"
    uv tool install --python 3.9 .
    cat > "$work/real/bwrap" <<'FAKE'
#!/bin/sh
printf '<%s>\n' "$@"
FAKE
    chmod 755 "$work/real/bwrap"
    cd "$work"
    export HOME="$work/home" PATH="$work/bin:$work/real:$PATH"
    command -v bwrap
    bwrap --dev /dev 'two words' ''
)
```

After the installed executable path, expect these four lines. The arguments
have no Codex command separator, so they pass through as supplied. Repeat
with `--python 3.14` to check the other interpreter.

```text
<--dev>
</dev>
<two words>
<>
```

## Distribution build and wheel installation

```sh
(
    set -eu
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    uv build --out-dir "$work/dist"
    export UV_TOOL_DIR="$work/tools" UV_TOOL_BIN_DIR="$work/bin"
    uv tool install "$work"/dist/*.whl
    cd "$work"
    export PATH="$work/bin:$PATH"
    command -v bwrap
    bwrap --version
)
```

`uv build` creates a source distribution and builds the wheel from it. The
wheel has the `py3-none-any` tag. To also verify the wheel with pip, run
`python -m pip install /absolute/path/to/bwrap_wrapper-0.1.0-py3-none-any.whl`
in a separate virtual environment with pip available. Change to a directory
outside the checkout and repeat the smoke test with that environment's
`bwrap`. [Hatchling wheel builder](https://hatch.pypa.io/dev/plugins/builder/wheel/)

## Ordinary tests

```sh
python3.9 -m unittest -v
python3.14 -m unittest -v
```

The runtime tests install `bwrap` with `uv tool install .` using the CPython
version running the suite. They use fake executables, synthetic files, and
injected mount readers. No namespace support is required. The Rust
comparisons and real isolation checks are opt-in.

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

The comparisons run the installed Python `bwrap` and the Rust `bwrap`.
The shared fixtures use equivalent JSON and TOML settings and the same fake
executable. Separate test drivers inject a synthetic mount table. The tests
compare forwarded bytes, exit or signal outcomes, and complete successful
logs with the timestamp and PID normalized. They check the project `/tmp`
bind in the Python result and compare the remaining arguments and log entries
against the Rust reference. Failure cases check diagnostic categories and
which log artifacts were created.

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

Set both variables to run both formats together. Each format runs through
the installed `bwrap` with workspaces inside and outside HOME and all
eight combinations of packages, memories, and `AGENTS.md` present or absent.
That is 16 cases per format. Fixtures are synthetic, with a read-only root
mount and writable binds into temporary directories. The checks cover HOME
hiding, generated helper execution and directory permissions, optional
sources, workspace writes, overlays, configured extra directories, unrelated
read-only mounts, and `/tmp` isolation and writes to the project cache. Real
WSL2 drive masking and end-to-end Codex sessions require target-host checks in
addition to this suite.
