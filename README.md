# bwrap-wrapper

A Python wrapper for Bubblewrap on Linux and WSL2. It adjusts Codex sandbox
mounts and forwards other invocations unchanged. Requires CPython 3.9 or newer,
`uv`, and a working Bubblewrap. Installing from source also requires a C compiler.

Run these commands from the checkout.

```sh
uv tool install .
export PATH="$(uv tool dir --bin):$PATH"
command -v bwrap
```

Verify that `command -v bwrap` shows the wrapper in uv's tool bin directory.
Keep the real Bubblewrap later in `PATH`. You can run `bwrap` from any directory.

- [Installation, settings, and mount behavior](docs/usage.md)
- [Smoke test, Rust comparison, and real isolation checks](docs/verification.md)
- [Apache-2.0 license](LICENSE) and [original attribution](NOTICE)
