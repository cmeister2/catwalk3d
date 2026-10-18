# Experimental GPU performance and qualification

Retaining uploaded geometry improved repeated rendering in the original
experiment. Fresh GPU jobs were slower and strict image qualification failed.
CPU remains the default. Automatic fallback handles unsupported input and
execution errors; it does not compare GPU output with a CPU image and cannot
detect these image-tolerance failures at runtime.

These are historical measurements of the prototype archived at
[`77a8de0`][archive], not measurements of the split PR. Source and executable
hashes are retained in the [Windows][windows-build] and [Linux][linux-build]
build records. The Windows matrix contains 162 jobs / 54 validated experiment
reports; the NAS matrix contains 81 jobs / 27 validated experiment reports.
Successful execution and report validation do not imply passing image quality.

Both platforms used 512×512 output, supersampling 1 and PNG encoding to memory
for timing. Fresh milliseconds below are medians of three complete child
launch-to-exit observations, including device/import setup and teardown, without
a PNG file write. Caches were not reset; fresh process does not mean cold cache.

| Host / backend | Input | Legacy CPU | GPU |
| --- | --- | ---: | ---: |
| Windows / DX12 | Gear | 127.85 | 346.64 |
| Windows / DX12 | NIST AP242 | 212.53 | 416.01 |
| Windows / DX12 | Repeated assembly | 717.98 | 765.80 |
| Windows / Vulkan | Gear | 130.55 | 371.99 |
| Windows / Vulkan | NIST AP242 | 201.85 | 416.12 |
| Windows / Vulkan | Repeated assembly | 690.49 | 776.69 |
| NAS / OpenGL | Gear | 26.65 | 229.93 |
| NAS / OpenGL | NIST AP242 | 222.24 | 588.11 |
| NAS / OpenGL | Repeated assembly | 1,093.32 | 1,983.02 |

The repeated assembly is a controlled ellipsoid mesh placed 64 times, giving
786,432 logical triangles. It has no textures, materials or authored normals
and is not production-representative. Retained-scene observations below repeat
the **same view, dimensions and settings**. They do not demonstrate different
cameras, multi-view generation or arbitrary new-model throughput.

| Windows backend | CPU median retained job, ms | GPU median retained job, ms | Shared CPU process / 3, ms | GPU process / 3, ms |
| --- | ---: | ---: | ---: | ---: |
| DX12 | 258.67 | 12.19 | 309.18 | 284.61 |
| Vulkan | 259.64 | 8.80 | 302.60 | 287.58 |

The job medians improve 21–30× versus legacy CPU, but the actual three-job
process improves only 5–8% versus shared CPU after setup and cleanup. Process / 3
is one measured setup/jobs/cleanup interval divided by three; it excludes process
startup and report serialization and is not three independent process runs.
Smaller Windows inputs remain slower over those three retained jobs. New-scene
assembly preparation still costs about 416–419 ms of CPU fitting/packing per
upload before the fast GPU draw. The [Windows report][windows-report] and
[NAS report][nas-report] preserve all scopes, inputs and regressions.

The original frozen comparator checks 512px CLI images at supersampling 1/2
with opaque/transparent backgrounds. Every supported GPU pair fails only
`interior_max_error`, whose limit is 3 RGBA channel values. Other geometry,
coverage, average-error and background gates pass; they do not override this
failure. All automatic-fallback pairs match encoded PNG and decoded RGBA exactly.

| Renderer configuration | Supported GPU pairs passing | Exact fallback pairs passing |
| --- | ---: | ---: |
| NAS / OpenGL | 0 / 12 | 8 / 8 |
| Windows / DX12 | 0 / 12 | 8 / 8 |
| Windows / Vulkan | 0 / 12 | 8 / 8 |

The [unchanged policy][policy], [NAS comparisons][nas-comparisons],
[DX12 comparisons][dx12-comparisons] and [Vulkan comparisons][vulkan-comparisons]
retain the failed results. The [pixel diagnosis][diagnosis] identifies rare
internal facet/occlusion edge coverage changes between CPU f64 rasterization
and native GPU rasterization. Smaller image tests permit nearby edge matches;
their passes do not qualify this stricter policy.

Native Windows used Radeon 780M through DX12 and Vulkan, with artifacts accessed
through WSL UNC paths; no WSL graphics was used. CPU/RSS/HWM measurements were
unavailable there. The NAS used Intel UHD 600 with Mesa/OpenGL, one CPU core and
a 512 MiB container cap under existing host memory pressure. It showed higher
GPU process/cgroup memory for every input. These are separate host observations,
not a controlled hardware comparison, and neither establishes a RAM saving.
The nominal explicit GPU allocation budget excludes CPU storage, driver
allocations and other live scenes; it is not a total process/system RAM cap.

[archive]: https://github.com/cmeister2/catwalk3d/tree/77a8de01aa7ae3074072c4ca0adb0558cf9934cd
[windows-build]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/builds/windows.json
[linux-build]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/builds/linux.json
[windows-report]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/recommendation.md
[nas-report]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/nas-recommendation.md
[policy]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/policy.json
[nas-comparisons]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/nas-results/comparisons.json
[dx12-comparisons]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/results/dx12/comparisons.json
[vulkan-comparisons]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/windows/results/vulkan/comparisons.json
[diagnosis]: https://github.com/cmeister2/catwalk3d/blob/77a8de01aa7ae3074072c4ca0adb0558cf9934cd/docs/research/gpu/2026-10-18/renderer/pixel-investigation/diagnosis.md
