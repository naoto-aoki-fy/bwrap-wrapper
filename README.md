# bwrap-wrapper

A standalone Python wrapper for Bubblewrap on Linux and WSL2. It adjusts Codex
sandbox mounts and forwards other invocations unchanged. Requires CPython 3.9 or
newer and a working Bubblewrap.

To install the single-file executable, run these commands from the checkout.

```sh
python3.9 build_zipapp.py --output-dir "$HOME/.local/lib/bwrap-wrapper/bin"
export PATH="$HOME/.local/lib/bwrap-wrapper/bin:$PATH"
```

Keep the real Bubblewrap later in `PATH`.

- [Installation options, settings, and mount behavior](docs/usage.md)
- [Smoke test, Rust comparison, and real isolation checks](docs/verification.md)
- [Apache-2.0 license](LICENSE) and [original attribution](NOTICE)
