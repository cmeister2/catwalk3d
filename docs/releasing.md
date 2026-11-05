# Releasing Catwalk

See the [README](../README.md) for installation and use, and the [contributing guide](https://github.com/cmeister2/catwalk3d/blob/main/CONTRIBUTING.md) for local builds, tests and gallery maintenance.

See the [release notes](release-notes.md) for migration guidance and published releases.

## Publication and versioning

Successful CI on `main` automatically publishes `catwalk3d` to crates.io and creates a matching GitHub release. Publication waits for `Success`. Conventional commits select the version: `fix:` produces a patch, `feat:` a minor release, and a breaking change a major release. Documentation-only commits produce no release.

`semantic-release-cargo` prepares the actual release version before the platform builds and publishes the crate; the GitHub plugin attaches the verified archives. Publication checks the complete target set, versions, source commits and archive/member checksums before tagging. Main workflows are serialized to keep version planning and publication together. Pull requests have one CI run, with superseded runs canceled.

Node dependencies support release tooling and browser builds and tests. Rust dependencies are published crates with no local Cargo patches.

## Binary archives

Each release includes `catwalk3d-VERSION-TARGET.zip` archives for these targets:

| Platform | Target |
| --- | --- |
| Linux x86-64, GNU | `x86_64-unknown-linux-gnu` |
| Linux ARM64, GNU | `aarch64-unknown-linux-gnu` |
| Linux x86-64, static musl | `x86_64-unknown-linux-musl` |
| Linux ARM64, static musl | `aarch64-unknown-linux-musl` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| Windows x86-64 | `x86_64-pc-windows-msvc` |

Every archive contains the executable, [README](../README.md), Catwalk's license, generated dependency and Rust runtime license notices, and `release.json` with its version, source commit, compiler and target. Static musl archives also include the musl copyright notice. The release includes individual `.zip.sha256` files, provenance JSON sidecars and a combined `SHA256SUMS` file.

## Maintainer configuration

Maintainers supply a crates.io token through the GitHub secret `CARGO_REGISTRY_TOKEN`; `PUBLISH_SECRET` is also accepted. Publication skips while neither is configured. Local `.env` files are excluded from Git and Cargo packages.
