# Native Windows renderer results and portable recommendation

Keep CPU as the default and GPU rendering experimental. Both native Windows backends completed the hardware workload, but **all 24 supported CPU/GPU image pairs failed the unchanged strict maximum-error gate**: 12 [DX12 pairs](results/dx12/comparisons.json) and 12 [Vulkan pairs](results/vulkan/comparisons.json). Each backend passed its eight exact CPU/auto fallback comparisons. Faster measured GPU draws do not establish qualified production acceleration.

The archived [Windows summary](results/windows-summary.json) validates all 54 experiment reports and 162 jobs from the [recorded matrix](manifest.json). The runner's 96 CLI command expectations passed, while the analyzer returned failure because of image qualification. Raw commands, observations, adapter capabilities and image comparisons are retained. The [Windows build record](../builds/windows.json) identifies the supplied native executables and source.

## Hardware and measurement scope

The actual wgpu 29.0.3 renderer selected AMD Radeon(TM) 780M, vendor `0x1002`, device `0x15bf`, `IntegratedGpu`. DX12 reported driver `32.0.31041.1004`; Vulkan reported the AMD proprietary driver, `26.8.1 (LLPC)`. These are native Windows MSVC executables and native Windows GPU APIs, not WSL graphics. The recorded Windows source commit is `ec68f04`, unchanged during the build. The recorded Linux and Windows source hashes differ only in `src/gpu/mod.rs`, corresponding to the documented test-only cleanup.

The build target was on NTFS. The measurement runner subsequently staged immutable executables, inputs and outputs in WSL `/tmp`, accessed by the Windows processes through UNC paths. Fresh process launch and import timings therefore include this harness's UNC filesystem costs; they are not measurements of a native NTFS installation. Outer PowerShell/interop orchestration wall time is recorded separately and excluded from the internal scopes below. UNC costs are not subtracted from those scopes.

All profiling uses 512×512 output, supersampling 1 and PNG encoding to memory, without writing PNG files. Fresh values are medians of three complete native child launch-to-exit observations, including startup, preparation, rendering, cleanup and report serialization. Reload job medians cover new import, expansion/upload, render, encoding and scene cleanup with one reused device. Process/3 divides the actual measured setup/jobs/cleanup interval by three, adding once-per-process device costs but excluding startup and report serialization. Each reload or retained process/3 value is one three-job process observation, not three independent process repetitions. Caches were not cleared and cases were not randomized.

## Fresh complete thumbnail work

Milliseconds, median of three child lifetimes:

| Backend | Input | Legacy CPU | Shared CPU | GPU |
| --- | --- | ---: | ---: | ---: |
| DX12 | Gear | 127.85 | 131.36 | 346.64 |
| DX12 | NIST AP242 | 212.53 | 216.90 | 416.01 |
| DX12 | Controlled repeated assembly | 717.98 | 415.61 | 765.80 |
| Vulkan | Gear | 130.55 | 126.57 | 371.99 |
| Vulkan | NIST AP242 | 201.85 | 205.16 | 416.12 |
| Vulkan | Controlled repeated assembly | 690.49 | 407.58 | 776.69 |

GPU fresh medians are slower than legacy CPU for every input on both backends: approximately 2.7–2.8× for gear, 2.0–2.1× for AP242 and 1.07–1.12× for the repeated assembly. The synthetic shared-CPU path instead reduces fresh elapsed time by approximately 41–42% versus legacy CPU. That improvement comes from representation/import/conversion behavior; it is not a GPU execution gain.

Initialization and complete scope matter. The printed synthetic fresh GPU job median is only 13.29 ms on DX12 or 12.24 ms on Vulkan, but excludes approximately 456–462 ms scene setup and 195–208 ms median device initialization. Its complete child lifetime is 766–777 ms. First fresh GPU launches are also slower than subsequent launches: gear DX12 `[881.81, 346.64, 333.66]`, Vulkan `[1011.46, 371.99, 344.11]` ms. Preserve these observations rather than treating a fresh process as a controlled cold-cache or steady-state measurement.

## New scenes with a reused device

Milliseconds. Job columns are medians of three jobs; process/3 includes the actual three-job process's device initialization and teardown:

| Backend | Input | CPU job | Shared CPU job | GPU job | CPU process/3 | Shared CPU process/3 | GPU process/3 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| DX12 | Gear | 46.97 | 46.81 | 46.71 | 51.33 | 53.65 | 162.83 |
| DX12 | NIST AP242 | 120.88 | 126.62 | 106.99 | 139.52 | 142.38 | 229.60 |
| DX12 | Controlled repeated assembly | 613.52 | 325.97 | 472.81 | 623.04 | 344.99 | 595.23 |
| Vulkan | Gear | 49.65 | 46.12 | 44.57 | 52.34 | 53.14 | 158.11 |
| Vulkan | NIST AP242 | 126.02 | 126.19 | 100.21 | 141.94 | 140.05 | 226.48 |
| Vulkan | Controlled repeated assembly | 591.15 | 341.15 | 472.48 | 606.65 | 357.81 | 598.62 |

GPU initialization in these reload processes is approximately 288–301 ms and device cleanup 17–27 ms. Consequently, AP242's lower median GPU job cost does not translate into a faster actual three-job process. The repeated assembly's GPU process is only 4.46% faster than legacy CPU on DX12 and 1.32% faster on Vulkan, while remaining 72.5% and 67.3% slower than shared CPU respectively. These small legacy-baseline differences do not establish a general new-scene GPU advantage.

The synthetic GPU upload's CPU preparation substage takes approximately 419 ms on DX12 or 416 ms on Vulkan, before approximately 2 ms GPU resource creation. Its median render is 6.64 or 8.89 ms. The substantial preparation cost remains despite the fast GPU draw. CPU rendering's own one-time geometry preparation is approximately 128 ms and its remaining raster/resolve stage approximately 122–124 ms. These are nested wall-time substages, not process CPU-time measurements, and must not be added again to their enclosing totals.

## Retaining an already uploaded scene

The retained runs repeat the same scene and render settings. They do not test changing cameras, changing dimensions or unrelated input files.

| Backend | Input | CPU median job, ms | GPU median job, ms | CPU process/3, ms | Shared CPU process/3, ms | GPU process/3, ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| DX12 | Gear | 5.39 | 3.66 | 24.36 | 25.73 | 133.37 |
| DX12 | NIST AP242 | 29.20 | 3.38 | 75.71 | 75.32 | 164.16 |
| DX12 | Controlled repeated assembly | 258.67 | 12.19 | 389.19 | 309.18 | 284.61 |
| Vulkan | Gear | 5.88 | 2.93 | 24.29 | 24.44 | 143.66 |
| Vulkan | NIST AP242 | 28.46 | 3.47 | 76.95 | 78.00 | 166.90 |
| Vulkan | Controlled repeated assembly | 259.64 | 8.80 | 386.74 | 302.60 | 287.58 |

GPU cached job medians are approximately 21× and 30× faster for the repeated assembly, but the measured three-job process is only 27% and 26% faster than legacy CPU, or approximately 8% and 5% faster than shared CPU. Device and scene setup explain the difference between these scopes. GPU is slower over three retained jobs for the two smaller inputs. Longer useful workloads could amortize setup differently, but were not measured here.

## Memory and image qualification

Native Windows process CPU, RSS and HWM are unavailable in this profiler and remain explicitly null in every summary row. They are not zero. No Windows CPU-consumption, peak-RAM or memory-saving conclusion is supported. There are no NAS cgroup limits or CPU-3 affinity restrictions on these Windows runs. The nominal 256 MiB explicit GPU budget is not a total RAM limit; estimated renderer allocations are approximately 4.08 MiB for gear, 4.55 MiB for AP242 and 6.54 MiB for the assembly, excluding driver allocations, CPU storage and incompletely measured integrated/shared memory.

The compact repeated assembly has a 353,400-byte structural geometry estimate versus 62,914,560 bytes expanded: one 6,146-position/12,288-triangle mesh placed 64 times, giving 786,432 logical triangles. Shared CPU then expands it for the legacy rasterizer. This proves a representation reduction, not a measured Windows whole-process memory reduction. The 586,109-byte generated input is a controlled workload without textures, materials or authored normals, not a production-representative CAD assembly.

Each backend's 20 image comparisons yields eight exact fallback passes and twelve GPU failures. Every GPU failure is solely `interior_max_error`; all other recorded gates pass. For both DX12 and Vulkan, maximum interior RGBA channel errors at samples 1/2 are gear **79/29**, AP242 **47/15**, and repeated assembly **14/6**, against the [frozen limit](../policy.json) of **3**. No alignment, policy relaxation or reclassification changes that result. The same mean-error/coverage passes do not override the strict maximum-error failure.

## Portable interpretation

The recorded results demonstrate actual renderer operation on native Windows DX12 and Vulkan on Radeon 780M, complementing the separate [NAS Intel/OpenGL evidence](../nas-recommendation.md). They do not establish support or performance on every GPU/driver, and provide no Metal validation. Wgpu's API portability does not remove backend feature/limit checks, CPU preparation, driver costs or image-qualification differences.

The practical recommendation remains CPU by default, shared/indexed representation as a separate useful optimization, and experimental GPU use only when its retained-scene tradeoff suits a measured workload. Resolve strict image qualification before promoting acceleration as production-ready. Any future reduction in CPU preparation must preserve numerical checks and CPU fallback. NAS and Windows have different hosts, affinity/resource constraints, drivers and filesystem paths, so their timings must not be presented as a controlled cross-hardware benchmark. No additional GPU execution or duplicate analyzer was launched for this independent interpretation.
