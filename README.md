# Catwalk

Turn 3D models into PNG thumbnails from your browser, the command line or a Rust application. Catwalk renders on the CPU with automatic framing, a fixed isometric view, simple lighting and antialiasing. Native builds run headlessly on Linux, macOS and Windows.

The Rust crate is **`catwalk3d`**; the command is **`catwalk`**.

[Create a thumbnail](https://cmeister2.github.io/catwalk3d/upload/) · [Browse example thumbnails](https://cmeister2.github.io/catwalk3d/) · [Check format coverage](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-coverage.md) · [Rust API reference](https://docs.rs/catwalk3d/)

## Install and make a thumbnail

Download a platform ZIP from [GitHub Releases](https://github.com/cmeister2/catwalk3d/releases), extract it, and open a terminal in the extracted directory. On Linux or macOS:

```sh
./catwalk model.stl --output thumbnail.png --size 256
```

On Windows, run `.\catwalk.exe model.stl --output thumbnail.png --size 256` in PowerShell.

Linux builds are available for x86-64 and ARM64 with GNU or musl. Choose musl for a statically linked executable, including Alpine and systems without glibc. GNU builds are compiled and tested on Ubuntu 24.04; glibc 2.39 or newer is the supported baseline. macOS builds cover Intel and Apple Silicon; Windows builds cover x86-64. The archives include license notices and release metadata, with SHA-256 checksums beside the downloads. macOS and Windows downloads are not signed or notarized.

To install with a current stable Rust toolchain, Cargo and a C compiler:

```sh
cargo install catwalk3d --locked
catwalk model.stl --output thumbnail.png --size 256
```

This writes a 256 × 256 PNG and fits the model into the frame. The default output is a 512 × 512 PNG on a light background.

![Example gear thumbnail rendered by Catwalk](examples/gear.png)

Running Catwalk requires no graphics driver, display server, CAD installation or subprocess converter.

## Create a thumbnail in your browser

The [browser tool](https://cmeister2.github.io/catwalk3d/upload/) reads a selected model, image or document and lets you download its PNG thumbnail. Selected files stay on your device. Choose automatic saved-preview preference or fresh geometry, output size and transparency. Add materials, textures or assembly parts stored beside the primary file as companions; the picker currently accepts files from one folder.

The page loads the compressed renderer when you generate a thumbnail and runs it in a worker. Browser limits are 64 MiB per file, 128 MiB total, 2,048 selected files and two minutes per job. Cancellation stops the worker; a failed input can be followed by another render. See the [format coverage guide](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-coverage.md) for supported versions and encodings.

## Which files can I preview?

Catwalk supports 91 extensions across mesh and CAD models, native application documents, point clouds, scientific volumes and images. Common inputs include STL, OBJ, 3MF, glTF/GLB, STEP, Blender, SketchUp, Rhino, 3ds Max and SolidWorks. Each reader supports a documented subset of its format.

See the [format coverage guide](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-coverage.md) for the complete compatibility table, supported versions, encodings and limitations. Run `catwalk --list-formats` for the extension list.

Use `catwalk --list-formats --json` for a versioned catalog with every extension alias, human-readable format descriptions and support categories. See the [discovery contract](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-catalog.md) and [JSON Schema](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-catalog.schema.json) for downstream CI checks and settings tables.

Models use a fitted static view. Supported colors and materials are preserved; OBJ, glTF/GLB and 3MF additionally support base-color textures. Animation files use a supported static pose or sample. Raster images are fitted to the canvas with their proportions preserved.

## Native documents and saved previews

By default, a supported saved image is used when available. Native documents with geometry readers fall back to fresh rendering when that image is missing or unreadable. A saved thumbnail can be blank or out of date. Choose the preview mode explicitly when needed:

```sh
# Regenerate a supported document's preview from stored geometry.
catwalk part.ipt --render-geometry --output fresh.png --size 256

# Extract a saved image at its original dimensions.
catwalk part.ipt --extract-saved-preview --output saved.png
```

`--render-geometry` ignores embedded images and requires a supported geometry reader. `--extract-saved-preview` uses the saved image's original dimensions and cannot be combined with canvas-size, transparency, background, color or sampling options. When the reader returns no saved image, extraction exits with code 3; corrupt or unsupported carriers can return an error.

Native assemblies can require companion files. See the [format coverage guide](https://github.com/cmeister2/catwalk3d/blob/main/docs/format-coverage.md) for each reader's requirements and limitations, plus the [native geometry examples](https://github.com/cmeister2/catwalk3d/blob/main/docs/native-geometry/README.md) and [native Autodesk examples](https://github.com/cmeister2/catwalk3d/blob/main/docs/native-autodesk/README.md).

## Output options

```sh
catwalk model.glb --output thumbnail.png --resolution 640,360
catwalk model.3mf --output thumbnail.png --transparent
catwalk model.obj --output - > thumbnail.png
```

On PowerShell versions before 7.4, use `--output thumbnail.png`; [binary stdout redirection is preserved from PowerShell 7.4 onward](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_redirection?view=powershell-7.5#redirecting-output-from-native-commands).

| Option | Effect |
| --- | --- |
| `--size 256` | Square PNG; default is 512 pixels |
| `--resolution 640,360` | Explicit width and height; `640x360` also works |
| `--transparent` | Transparent background with antialiased edges |
| `--background '#f5f7fa'` | Background color; shown value is the default |
| `--color '#64a0dc'` | Fallback color for geometry without material or vertex colors |
| `--samples 1` | Supersampling factor from 1 to 4; default is 2 |
| `--output -` | Write PNG bytes to stdout |

Use `catwalk --help` for all options. `--no-background` aliases `--transparent`, and `--list-readers` aliases `--list-formats`.

CLI dimensions are 1–4096 pixels per axis. Geometry rendering allows at most 16 million supersampled pixels; reduce dimensions or use `--samples 1` for a very large thumbnail. Inputs and referenced resources have size limits, with additional limits for individual readers documented in the coverage matrix.

Errors are written to stderr and return a nonzero exit code. Decode failures occur before the destination is opened, preserving an existing thumbnail. For an unsupported file variant, check its coverage entry and export to a supported encoding or geometry format. Unsupported native document variants may still need a new saved thumbnail from the authoring application.

## Use from Rust

Add the dependency:

```toml
[dependencies]
catwalk3d = "2"
```

```rust,no_run
use catwalk3d::{render::{RenderOptions, write_png}, thumbnail};
use std::{fs::File, path::Path};

fn main() -> catwalk3d::Result<()> {
    let options = RenderOptions {
        width: 256,
        height: 256,
        transparent: true,
        ..Default::default()
    };
    let image = thumbnail(Path::new("model.stl"), &options)?;
    write_png(&image, File::create("thumbnail.png")?)?;
    Ok(())
}
```

`thumbnail` follows the same saved-image preference and geometry fallback as the CLI. `geometry_thumbnail` forces stored-geometry rendering. For supported documents, `saved_preview::extract` returns an image at its original dimensions, or `None` when the reader finds no saved image; malformed or unsupported carriers can return an error. `saved_preview::inventor::extract_png` preserves native PNG bit depth and strips ancillary metadata.

For custom scenes, `formats::load` produces a `scene::Scene` containing triangles, points, line segments and supported appearances. Pass it to `render::render`, or construct your own scene. `write_png` accepts any `std::io::Write`, including buffers and stdout. See the [API reference](https://docs.rs/catwalk3d/) for the types and functions.

See the [release notes](https://github.com/cmeister2/catwalk3d/blob/main/docs/release-notes.md#upgrading-to-2x) for Rust API and rendering changes when upgrading to 2.x.

## Contributing

See [CONTRIBUTING.md](https://github.com/cmeister2/catwalk3d/blob/main/CONTRIBUTING.md) for building from source, browser development, tests and snapshot updates. The [release guide](https://github.com/cmeister2/catwalk3d/blob/main/docs/releasing.md) describes release automation, artifacts and maintainer setup.

## License

Catwalk's code and authored fixtures are MIT licensed. Retained upstream test assets keep their separately documented licenses and notices, including the AGPL-3.0 slicer examples. Catwalk is an independent thumbnail implementation.
