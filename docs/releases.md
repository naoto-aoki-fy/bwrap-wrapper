# Releases

The [Release workflow](../.github/workflows/release.yml) runs on pushes to `main`.
You can also run it from the Actions tab using **Run workflow** on `main`.
Runs selected on other branches are skipped. No personal access token is needed;
the release job uses `GITHUB_TOKEN` with `contents: write`. Repository rules must
allow that token to create version tags and releases.

Each run tests and builds with stable Rust on native Ubuntu x86_64 and ARM64
runners, targeting `x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`. Both builds must succeed before publication.

Immediately before publication, the workflow fetches all tags and selects the
numerically highest exact `vMAJOR.MINOR.PATCH` tag, regardless of which branch
contains it. It increments only the patch component (for example, `v1.9.12`
becomes `v1.9.13`). With no matching tags it starts at `v0.0.1`. Prerelease tags,
build metadata, leading zeroes, and other names are ignored. This release version
is independent of the package version in `Cargo.toml`.

Release runs are serialized to avoid selecting the same tag concurrently.
GitHub may replace older pending runs when several pushes arrive while a release
is running. The workflow tags the exact commit that was built, pushes the tag,
and publishes a GitHub release with generated notes and these assets:

- `bwrap-x86_64-unknown-linux-musl`
- `bwrap-aarch64-unknown-linux-musl`

Only the two static executables are uploaded as release assets.
Download the executable matching your architecture and make it executable with
`chmod +x bwrap-<target>`. Follow the README installation instructions,
using the downloaded executable instead of `target/release/bwrap`.
The real Bubblewrap executable is still required.

If publication fails after the tag push, the tag remains on GitHub. Inspect the
failed run and finish publishing that tag if needed; a fresh run selects the next
patch version.
