# Shared STEP geometry: measured motivation

The opt-in `formats::load_instanced` API retains reusable STEP meshes and their
placements. Callers can inspect the compact scene or expand it with `to_scene`
for the existing CPU renderer. The production thumbnail CLI and `formats::load`
keep their existing import path. See [the API guide](shared-geometry.md).

The measurements below come from the earlier GPU experiment, archived at
[`77a8de0`][archive], before this work was separated into reviewable PRs. They
motivate the shared representation; they are **not measurements of this split
PR**. The original CPU and shared-CPU modes used the same GPU-enabled
`profile_gpu` executable, with no GPU rendering in either mode. The [Windows
build record][windows-build] identifies source `ec68f04`; the [Linux build
record][linux-build] identifies `6e00d13`.

The controlled repeated assembly stores one ellipsoid mesh and 64 placements.
It has no textures, materials or authored normals and is not a representative
production CAD assembly. The deterministic [generator][generator] and input
hashes in the [Windows manifest][windows-manifest] identify the workload.

| Repeated-assembly quantity | Value |
| --- | ---: |
| Source positions / source triangles | 6,146 / 12,288 |
| Occurrences / logical triangles | 64 / 786,432 |
| Compact structural geometry estimate | 353,400 bytes |
| Expanded triangle payload estimate | 62,914,560 bytes |

These are structural payload estimates, excluding allocator overhead, temporary
tables and the rest of the process. Shared CPU still expands the scene before
rasterization. The estimates do not establish a reduction in peak process RAM.

Fresh native Windows child lifetimes below are medians of three observations,
in milliseconds, at 512×512 with supersampling 1 and PNG encoding to memory.
They include launch, import, expansion, rendering, cleanup and report
serialization, but no PNG file write. DX12/Vulkan identify the surrounding
benchmark batches; both columns here execute entirely on the CPU.

| Windows batch | Input | Legacy CPU | Shared CPU | Elapsed-time change |
| --- | --- | ---: | ---: | ---: |
| DX12 | Gear OBJ | 127.85 | 131.36 | 2.7% higher |
| DX12 | NIST AP242 | 212.53 | 216.90 | 2.1% higher |
| DX12 | Repeated assembly | 717.98 | 415.61 | 42.1% lower |
| Vulkan | Gear OBJ | 130.55 | 126.57 | 3.0% lower |
| Vulkan | NIST AP242 | 201.85 | 205.16 | 1.6% higher |
| Vulkan | Repeated assembly | 690.49 | 407.58 | 41.0% lower |

The assembly improvement depends on the host and workload. For a process
importing three new copies of that input, the measured setup/jobs/cleanup
interval divided by three gives the following milliseconds per job. This is one
three-job process per row, excluding process startup and report serialization;
it is not three independent process repetitions.

| Host / batch | Legacy CPU | Shared CPU | Elapsed-time change |
| --- | ---: | ---: | ---: |
| Windows / DX12 | 623.04 | 344.99 | 44.6% lower |
| Windows / Vulkan | 606.65 | 357.81 | 41.0% lower |
| Synology DS920+ / Linux | 1,069.15 | 1,102.82 | 3.1% higher |

The NAS compact import took about 39–40 ms versus 90–93 ms for expanded import,
but expanding the shared scene added about 84 ms. Similar peak process RSS on
the NAS and unavailable Windows RSS measurements prevent a measured RAM-saving
claim. Retaining instances through CPU rasterization could avoid expansion;
that optimization is not implemented here.

Windows ran native MSVC executables on the Radeon 780M host, with artifacts
accessed through WSL UNC paths. The lifetimes include those filesystem costs.
The NAS used one CPU core and a 512 MiB container limit under existing host
memory pressure. Caches were not reset, cases were not randomized, and these
hosts are not a controlled hardware comparison. Individual observations,
resource scopes and regressions remain in the [Windows report][windows-report],
[Windows summary][windows-summary] and [NAS report][nas-report].

For new CPU-only measurements, generate the assembly with
`python3 tools/generate-instanced-workload.py --output /tmp/repeated.step`, then
compare identical inputs and settings:

```sh
cargo run --release --locked --example profile_thumbnail -- /tmp/repeated.step /tmp/legacy.json --fresh-process --iterations 3 --size 512 --samples 1
cargo run --release --locked --example profile_thumbnail -- /tmp/repeated.step /tmp/shared.json --fresh-process --iterations 3 --size 512 --samples 1 --shared-import
```

This example is a separate harness. Record new results with this PR's source
and binary hashes rather than attributing them to the earlier experiment.

[archive]: https://github.com/cmeister2/catwalk3d/tree/77a8de01aa7ae3074072c4ca0adb0558cf9934cd
[windows-build]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/builds/windows.json
[linux-build]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/builds/linux.json
[generator]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/tools/generate-gpu-workload.py
[windows-manifest]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/manifest.json
[windows-report]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/recommendation.md
[windows-summary]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/results/windows-summary.json
[nas-report]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/nas-recommendation.md
