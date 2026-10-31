# Shared meshes and STEP instances

`formats::load_instanced` imports a scene as reusable indexed meshes and ordered
instance placements. STEP imports retain each source geometry item once and store
its assembly placements separately. This can avoid repeated geometry storage
while callers inspect or retain a large assembly.

The API is available without graphics dependencies. The existing `Scene`,
`formats::load`, thumbnail functions and command-line defaults continue to use
their existing expanded CPU path. Select shared import explicitly in Rust:

```rust
use catwalk3d::{Result, formats, render};
use std::{fs::File, path::Path};

fn main() -> Result<()> {
    let shared = formats::load_instanced(Path::new("assembly.step"))?;
    let expanded = shared.to_scene()?;
    drop(shared);
    let image = render::render(&expanded, &render::RenderOptions::default())?;
    render::write_png(&image, &mut File::create("thumbnail.png")?)?;
    Ok(())
}
```

Other formats currently adapt their existing imported scene to an indexed mesh
with an identity placement. Points and lines stay in world coordinates. This
adapter preserves their geometry and appearances; it does not preserve source
instancing from other formats.

Positions remain `f64` and share storage only when their coordinate bits match.
Triangle order, flat colors and corner appearances remain separate from position
indices, preserving material, normal and texture-coordinate seams. Instances use
row-major affine transforms. Expansion transforms authored normals with the
inverse transpose; singular transforms omit authored normals so the CPU renderer
can derive geometric normals. Explicit reversed winding swaps all corner
attributes together.

Validation checks finite geometry, references, affine placements and appearance
limits. The five-million-triangle expansion limit counts every placed occurrence,
even when its source mesh is stored only once. STEP parsing and traversal retain
their existing cumulative resource checks. Shared import can reject geometry that
cannot fit the indexed representation or its resource limits.

The current CPU renderer consumes expanded triangles, so `to_scene` allocates that
representation. Drop the shared scene before rendering when it is no longer
needed. A compact geometry representation does not establish a reduction in peak
process or system memory, and performance depends on the input and platform.
The [measured motivation](shared-geometry-performance.md) records the predecessor
experiment's improvements, regressions and measurement limits.

To compare CPU paths without GPU dependencies:

```sh
python3 tools/generate-instanced-workload.py --output /tmp/repeated.step
cargo run --release --locked --example profile_thumbnail -- /tmp/repeated.step legacy.json --fresh-process --iterations 3 --size 512 --samples 1
cargo run --release --locked --example profile_thumbnail -- /tmp/repeated.step shared.json --shared-import --fresh-process --iterations 3 --size 512 --samples 1
```

The pure Python generator creates a controlled 64-instance STEP assembly with
786,432 logical triangles and a JSON witness of its geometry and placements.
It is a synthetic representation workload without materials or textures. The
generator preserves its original header and metadata so the generated STEP
remains identical to the earlier experimental workload.

The example also supports `--reuse-scene` for repeated rendering and reloads the
scene for each job by default. `--shared-import` applies in every mode and is
forwarded to fresh child processes. Reports identify the selected import path;
its import phase includes conversion and release of the compact scene. Fresh
processes retain filesystem caches. Linux CPU/RSS observations are sampled and
Windows resource fields are unavailable; neither is a total memory measurement.
