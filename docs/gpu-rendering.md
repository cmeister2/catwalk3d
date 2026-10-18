# Experimental native GPU rendering

Build an optional hardware renderer with a native Rust toolchain:

```sh
cargo build --release --locked --features gpu
./target/release/catwalk model.step --output thumbnail.png --renderer gpu
./target/release/catwalk model.step --output thumbnail.png --renderer auto
```

CPU rendering remains the default. `--renderer gpu` requires a supported hardware
adapter and returns an error for unsupported geometry or settings. `auto` permits
complete-scene CPU fallback when hardware, precision, resource limits or scene
features prevent GPU rendering. Builds without the GPU feature keep `auto` on the
CPU and explain the missing feature when `gpu` is explicitly required. Saved
document previews and raster image decoding retain their normal preference.

The GPU feature is native only. Browser/WASI builds retain the existing CPU path.
wgpu abstracts Vulkan, DirectX 12, Metal and OpenGL/EGL; a compatible installed
driver is still required. `--gpu-backend auto|vulkan|dx12|metal|gl` and
`--gpu-adapter NAME` control selection for diagnosis. CPU/software adapters and
unverified adapter types are rejected by the hardware path.

## Qualified scope

The prototype renders opaque untextured surfaces, including legacy flat colors,
linear material colors, corner colors, authored normals, unlit materials,
single/double-sided materials and affine mirrored placements. Textures, alpha
masking/blending, point/line primitives and the dedicated XY drawing camera use
CPU fallback. Singular or insufficiently precise transforms also require CPU
fallback. Subpixel legacy surfaces that need the CPU renderer's outline pass are
not qualified for strict GPU rendering. Unsupported cases never silently omit
part of the scene.

Placement, projection, normal transformation, lighting, rasterization and box
supersampling run in shaders. Parsing, topology validation, tessellation and
double-precision camera fitting remain on the CPU. Precision checks reject GPU
placements that move corners by more than 0.05 supersampled pixels, normal
directions that exceed the documented validation tolerances, or placements that
change the CPU renderer's near-degeneracy classification.
Distinct source placements sharing a local coordinate basis that collapse to
identical packed projection rows request CPU fallback, preserving a demonstrated
near-coplanar ordering case. This conservative check does not establish equivalent
depth precision for arbitrary near-coincident surfaces. Such geometry, including
near-coincident faces within one mesh, remains outside the qualified scope.

STEP loading preserves each source part as one indexed mesh plus occurrence
transforms. Other importers initially adapt their existing expanded scenes.
Position indexing uses exact bits; GPU vertices split at complete attribute
boundaries. The existing public `Scene`, `formats::load` and CPU rendering APIs
remain available. `formats::load_instanced` and the `instanced` module expose the
shared representation independently of GPU support.

## Reuse and resource accounting

Rust callers can retain `gpu::GpuRenderer` for its device and pipelines, call
`upload` to obtain a reusable `gpu::GpuScene`, and call `render` repeatedly. The
uploaded scene retains shared mesh buffers and instance transforms. Prepared CPU
vertex arrays are released after upload; callers choose whether to retain their
source scene. Intermediate geometry stays on the device; only the resolved
thumbnail is read back.

`--gpu-memory-mib` defaults to 256 and limits the nominal explicit buffers and
textures requested for one uploaded scene. It excludes CPU scenes, temporary
packing tables, driver allocations and other live GPU scenes. It is not a total
process or system memory cap. An integrated GPU shares system RAM. Process RSS and
explicit GPU bytes must not be added together as though they were independent
measurements of total memory. A rejected GPU budget can lead `auto` to a CPU path
with different memory requirements.

## Measurements and current evidence

The [goal](gpu-acceleration-goal.md) defines correctness and measurement criteria.
[Hardware probes and NAS CPU baselines](research/gpu/2026-10-18/README.md) establish
available hardware and reference costs. They do not establish GPU thumbnail
speedups. The standalone capability probe uses wgpu 30; this renderer uses the
fully published wgpu 29 backend family so its optional Metal dependency resolves.
Runtime validation of the actual renderer is tracked separately from probe passes.
The [initial NAS hardware image validation](research/gpu/2026-10-18/renderer/nas-correctness-initial/metadata.json)
passed all 15 cases on Intel UHD 600. That intermediate source qualification is
separate from final-build validation and benchmark results; it does not establish
performance benefits or arbitrary near-coincident depth precision.

Build the comparable profiling harness with:

```sh
cargo build --release --locked --features gpu --example profile_gpu
./target/release/examples/profile_gpu model.step --mode cpu --report cpu.json
./target/release/examples/profile_gpu model.step --mode shared-cpu --report shared-cpu.json
./target/release/examples/profile_gpu model.step --mode gpu --report gpu.json
```

The same executable and image settings compare legacy CPU, shared import with CPU
expansion, and shared GPU rendering. Setup/import/upload and repeated render/PNG
costs are separate. `--reload-scene` imports and prepares a new scene for each job
while retaining the GPU device and pipelines; the default retains uploaded
geometry too. Add `--fresh-process` to measure complete independent jobs. These
modes retain filesystem and driver caches. GPU mode refuses CPU
fallback. `tools/generate-gpu-workload.py` generates a deterministic repeated STEP
assembly to isolate representation effects; it supplements real-format workloads
and is not evidence of production-wide performance.

CPU reports split global validation/normalization/camera fitting from
rasterization, per-triangle preparation and resolve. GPU upload reports split CPU
geometry preparation from resource creation wall time. Resource creation does not
wait for a distinct DMA-completion fence; the first draw may complete pending
transfers. Import still includes decoding, validation and format tessellation.
