# Portable GPU acceleration goal

Enable Catwalk to use available GPU hardware through an optional `wgpu` backend,
while preserving the existing CPU renderer and format support. The target is
portable acceleration across supported GPU systems, including native Windows
workstations and constrained Linux/NAS deployments. The Synology DS920+ is a
resource-constrained validation target, not the architecture's defining platform.

## Current evidence

- Native Windows passed the same `wgpu` 30.0.1 graphics and compute probe on
  AMD Radeon(TM) 780M hardware, PCI vendor `0x1002`, device `0x15BF`. An explicit
  DX12 run used driver `32.0.31041.1004`; automatic backend selection chose
  Vulkan with the AMD proprietary driver. Both verified triangle pixels and all
  components of 4,096 transformed vec4 values. The native Windows executable was
  launched through WSL interop; these were Windows GPU runs, not WSL2 rendering.
- The NAS passed the `wgpu` 30.0.1 GL graphics and compute probe in an isolated
  container with a 256 MiB memory limit. The selected hardware adapter was
  `Mesa DRI Intel(R) UHD Graphics 600 (GLK 2)`, reported as `IntegratedGpu`, using
  OpenGL 4.6 / Mesa 21.3.9 Amber. The probe verified triangle pixels and every
  component of 4,096 transformed vec4 values, and exited successfully. This
  demonstrates those workloads on that configuration, not Catwalk throughput or
  a general 256 MiB bound on graphics/system memory.

The reusable [hardware probe](../tools/gpu-probe/README.md) and
[recorded results](research/gpu/2026-10-18/README.md) retain adapter and capability
details. These compatibility checks do not establish Catwalk performance benefits.

## Portability and fallback

- Let `wgpu` abstract supported graphics APIs and discover adapters. Catwalk must
  still choose an adapter, inspect its capabilities, request supported features
  and limits, and handle initialization or execution failures.
- Report the selected adapter, backend, device type, features and relevant limits.
  Distinguish hardware adapters from software implementations. A successful
  hardware-only probe establishes compatibility, not useful performance.
- Do not assume that API abstraction supplies drivers, makes every GPU compatible,
  gives every backend identical features, or guarantees speed or memory savings.
- Keep the CPU path available without a graphics device. Unsupported capabilities
  or unavailable devices must permit a complete CPU fallback. If neither path can
  satisfy resource limits, return an actionable error rather than partial output.
- Keep backend and adapter overrides available for diagnosis. Validate advanced
  compute paths separately from the basic rendering path.

## Scene and computation design

- Introduce a shared, indexed scene representation with reusable meshes and
  instance transforms. Keep the public legacy scene/API compatible through
  adapters and a checked CPU expansion path. Start with repeated STEP parts;
  extend to glTF and native assemblies after preserving their distinct semantics.
- Avoid expanding every occurrence into separate transformed CPU triangles before
  GPU rendering. Retain corner attributes and material boundaries when indexing;
  never merge distinct objects or weld vertices by approximate position.
- Move transforms, projection, lighting and sampling into shaders. Evaluate
  further parallel work where measurements justify it: deformation, bounds and
  normal calculations, volume filtering, and bounded curve/surface evaluation.
  Keep intermediate results on the device where practical and read back the final
  thumbnail rather than every intermediate array.
- Initially retain parsing, reference/topology validation and robust CAD
  tessellation on the CPU. Preserve double-precision validation, coordinate
  normalization, placement and tolerance decisions before preparing GPU data.
  General trimmed CAD meshing is a separate undertaking from surface evaluation.
- Preserve authored visibility, units, instance placement, mirrored winding,
  normal transforms, material/texture conventions, transparency and point/line
  behavior. Route unsupported cases through the complete CPU path.
- Reuse the GPU device, pipelines and reusable resources across jobs where the
  calling API allows it. Measure fresh-process CLI use and reused-device use
  separately; neither may stand in for the other.

## Evidence and implementation phases

1. **Baseline:** measure representative small meshes, repeated assemblies,
   textured/transparent scenes and CAD-heavy inputs. Separate import, scene
   preparation, rendering, encoding and total elapsed time; record memory and
   output settings. Existing workstation software-rendering comparisons are a
   baseline, not evidence of a GPU speedup or NAS performance.
2. **Compatibility:** probe actual hardware adapters and offscreen rendering on
   native Windows and the NAS. Test missing devices and insufficient capabilities
   as well. The Windows target is a native workstation run, not WSL2 GPU
   troubleshooting. Retain adapter/backend/driver details with each result.
3. **Renderer:** implement a small optional GPU path with deterministic camera
   fitting and image readback. Qualify opaque geometry first, then extend feature
   coverage while retaining CPU fallback and existing defaults during evaluation.
4. **Shared geometry:** load repeated STEP parts once and draw instances. Check
   independently authored placements, mixed units, normals, coincident distinct
   occurrences and existing importer refusals. Compare against legacy expansion.
5. **Broader acceleration:** add shader operations and importer paths only with
   correctness checks and measurements of complete thumbnail jobs. Expand the
   device/backend matrix as access permits and label untested combinations.

## Acceptance criteria

- Existing CPU use, saved-preview behavior and supported formats keep working on
  machines with no usable GPU. GPU failures do not silently omit geometry.
- Native Windows hardware and the constrained NAS each have recorded
  compatibility results, with explicit capabilities and qualified feature scope.
  Results distinguish unsupported configurations from implementation failures.
- Shared scenes retain geometry once per reusable mesh and placements once per
  occurrence. Their checked CPU expansion matches existing geometry expectations;
  GPU images meet documented framing, geometry, color and alpha tolerances.
- Benchmarks report identical inputs, output dimensions and quality settings,
  warmup policy, setup/upload/readback costs, CPU time and total elapsed time.
  No speedup is claimed from hardware availability or isolated shader timings.
- Memory results include process peak RAM, GPU allocations where observable and
  system memory pressure. On integrated GPUs, account for shared system RAM and
  avoid double-counting pages. Process RSS alone cannot establish total savings.
- Demonstrate a measured benefit on representative work before recommending the
  GPU path for that workload. Report regressions and resource tradeoffs; do not
  promise acceleration or lower memory use for every model or device.

This document defines the clarified goal and required evidence. It does not
record an implemented backend or establish a measured performance improvement.
