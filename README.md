# bwrap-wrapper-rs

A standalone Rust wrapper for Bubblewrap on Linux and WSL2. It adjusts Codex
sandbox mounts, logs each invocation, and replaces itself with the real `bwrap`.
Other invocations retain their arguments.

Requires stable Rust 1.85+ (edition 2024), a linker, and Bubblewrap for actual use.
Build and install into a separate directory, keeping the real executable intact:

```sh
cargo build --release
install -Dm755 target/release/bwrap "$HOME/.local/lib/bwrap-wrapper/bin/bwrap"
export PATH="$HOME/.local/lib/bwrap-wrapper/bin:$PATH"
codex
```

`HOME` is required. Logs include the entire environment and can contain secrets.
The wrapper searches PATH for the first executable `bwrap` other than itself.

- [Usage, environment variables, mount behavior, and logging](docs/usage.md)
- [Safe smoke test using a fake executable](docs/smoke-test.md)
- [Automated releases and musl binaries](docs/releases.md)
- [Compatibility audit, tests, project structure, and limitations](docs/compatibility.md)

Ported from the Apache-2.0 **bwrap-wrapper** Python project at revision
`e614a089b80b6f7832edd66950bba8f06abe0040`, supplied in `./python` for comparison.
The original [Apache-2.0 license](LICENSE) is retained. Python is only needed for
the optional development comparison, never to run the installed Rust executable.
