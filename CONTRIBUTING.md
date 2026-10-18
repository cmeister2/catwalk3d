# Contributing to Catwalk

See the [README](README.md) for installation and everyday use, the [format coverage](docs/format-coverage.md) for supported reader subsets, and the [release guide](docs/releasing.md) for publication and binary packaging.

## Build from source

Install a current stable Rust toolchain with Cargo and a C compiler. The importers and renderer use Rust implementations; Cadmpeg's bundled Zstandard compression requires the C compiler when building from source. Running Catwalk requires no graphics driver, display server, CAD installation or subprocess converter.

Clone the repository and preview an included model:

```sh
git clone https://github.com/cmeister2/catwalk3d.git
cd catwalk3d
cargo run --release --locked -- examples/models/gear.obj --output thumbnail.png --size 256
```

## Browser development

Install Node.js LTS, Python 3, Clang with WebAssembly support and the Rust WASI target, then build the static page from the repository root:

```sh
rustup target add wasm32-wasip1
npm ci --ignore-scripts
CC_wasm32_wasip1=clang npm run build:web
python3 -m http.server 4173 --bind 127.0.0.1 --directory target
```

Open `http://127.0.0.1:4173/web/`. The build writes the page to `target/web` and keeps Cargo output in `target/web-build`.

After building, run the WASI runner tests and install Chromium for the browser upload and download tests:

```sh
npm run test:web:node
npx playwright install chromium
npm run test:web:browser
```

Playwright starts the local HTTP server automatically or reuses an existing one. CI supplies runtime notices, verifies the page, and publishes it beside the [example gallery](https://cmeister2.github.io/catwalk3d/) at `/upload/` on successful `main` builds.

## Tests and example gallery

Published Cargo packages exclude test fixture files and snapshots. Small library unit-test inputs are authored in memory; real-file checks run as integration tests from a Git checkout. CI also runs the library tests from the extracted crate to verify they work without repository fixtures.

Run these checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

CI checks formatting, strict Clippy, documentation and the publishable crate, then tests on Linux, macOS and Windows. It also compiles all seven release targets, runs every-extension image comparisons using each optimized executable, and tests the executable extracted from each archive. The aggregate job is named `Success`; it passes only when the checks, platform tests, gallery and binary preparation/builds pass. Linux tests run with display-related environment variables unset.

The [format corpus](tests/fixtures/formats/manifest.json) includes an encoded input for every advertised extension, plus additional encoding and scene examples. `cargo test --locked --test format_snapshots` renders each case through the CLI and compares decoded pixels with [committed PNG snapshots](tests/snapshots/formats). Missing fixtures, decoding failures and visual changes fail CI. Transparent pixels use premultiplied RGBA comparison, with a maximum per-channel tolerance of 2/255 for platform rounding.

CI retains actual, expected and amplified difference images for each platform. Successful runs publish the tested images to the [gallery](https://cmeister2.github.io/catwalk3d/), where you can search formats and compare snapshots. Saved-image fixtures include licensed original projects and authored image carriers; an image-carrier test alone establishes no geometry-rendering coverage.

For a local gallery after testing, run `cargo run --locked --example gallery -- --report target/format-snapshots` and serve `target/gallery` with a static HTTP server. To intentionally update baselines, use `cargo run --locked --example gallery -- --update-snapshots`, inspect the images and commit the PNG changes. Normal tests never update baselines. Authored inputs can be regenerated with `cargo run --locked --example generate_fixtures`.
