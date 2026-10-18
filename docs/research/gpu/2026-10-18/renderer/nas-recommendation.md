# Independent NAS renderer interpretation

Keep CPU as the NAS default. **All 12 supported CPU/GPU image pairs failed the unchanged strict quality gate.** The eight CPU/auto fallback comparisons passed exactly. The GPU discrepancy is under investigation; timings below describe an experimental renderer and do not establish qualified production acceleration.

The archived [profile summary](nas-results/profile-summary.json) validates 27/27 experiment reports and 81 jobs against the [finalized manifest](nas-results/manifest-all.json), deployed hashes and [Linux build record](builds/linux.json). The actual renderer used wgpu 29.0.3 on Intel UHD Graphics 600, OpenGL/Mesa 21.3.9 Amber. The [image comparisons](nas-results/comparisons.json) apply the predeclared [policy](policy.json) without alignment or rescaling. Archived reports were independently summarized again successfully: 27/27 valid, verified provenance, no global validation errors.

Retaining an uploaded scene showed lower measured render costs, but these results do not establish faster fresh thumbnail processes, cheaper new-scene processing, or lower total memory use.

## Strict image qualification

All 12 GPU pairs failed only `interior_max_error`: the permitted maximum absolute RGBA channel error is 3. Every other recorded gate passed, including dimensions, coverage, bounds, centroid, interior mean error and distant-background error. The mean below is the mean absolute error across RGBA channels over the policy's interior mask. Low averages do not replace the maximum-error requirement.

| Input | Samples | Background | Interior max error | Interior mean error | Failed criterion |
| --- | ---: | --- | ---: | ---: | --- |
| Gear | 1 | Opaque | 79 | 0.000435 | `interior_max_error` |
| Gear | 1 | Transparent | 79 | 0.000435 | `interior_max_error` |
| Gear | 2 | Opaque | 29 | 0.000366 | `interior_max_error` |
| Gear | 2 | Transparent | 29 | 0.000366 | `interior_max_error` |
| NIST AP242 | 1 | Opaque | 47 | 0.001182 | `interior_max_error` |
| NIST AP242 | 1 | Transparent | 47 | 0.001182 | `interior_max_error` |
| NIST AP242 | 2 | Opaque | 15 | 0.000512 | `interior_max_error` |
| NIST AP242 | 2 | Transparent | 15 | 0.000512 | `interior_max_error` |
| Controlled repeated assembly | 1 | Opaque | 14 | 0.004268 | `interior_max_error` |
| Controlled repeated assembly | 1 | Transparent | 14 | 0.004268 | `interior_max_error` |
| Controlled repeated assembly | 2 | Opaque | 36 | 0.003004 | `interior_max_error` |
| Controlled repeated assembly | 2 | Transparent | 36 | 0.003004 | `interior_max_error` |

## Complete jobs and initialization

All values below are milliseconds at 512×512, supersampling 1. Fresh columns are medians of three complete child launch-to-exit observations, including startup, teardown and report serialization. Reload job columns are medians of three new-scene jobs with one reused device; each includes import, expansion/upload, rendering, PNG encoding to memory, and scene cleanup. The last columns divide the measured reload process setup/jobs/cleanup interval by three, including once-per-process device costs but excluding startup and report serialization. Profiling encodes PNGs in memory; neither scope includes writing a PNG file. Each last-column value comes from one three-job process, not three independent process repetitions.

| Input | Fresh CPU | Fresh GPU | Reload job CPU | Reload job GPU | Reload process / 3 CPU | Reload process / 3 GPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Gear OBJ, 512 triangles | 26.65 | 229.93 | 24.51 | 28.23 | 23.93 | 49.41 |
| NIST AP242, 3,388 triangles | 222.24 | 588.11 | 212.04 | 188.03 | 212.86 | 210.20 |
| Controlled repeated assembly, 786,432 logical triangles | 1,093.32 | 1,983.02 | 1,068.59 | 1,792.86 | 1,069.15 | 1,809.70 |

Fresh GPU medians are 8.63×, 2.65× and 1.81× the corresponding CPU times. Every observed GPU fresh child was slower than its corresponding CPU observation, including the third launch. Full fresh observations must be used instead of the printed fresh job medians: for example, the synthetic GPU job is 62.84 ms only after its approximately 1,739 ms scene setup and 148 ms median device initialization have been excluded.

There are substantial first-launch outliers, which must remain visible. Ordered GPU fresh launch-to-exit observations were gear `[8355.85, 229.93, 102.30]`, AP242 `[5745.25, 588.11, 269.35]`, and synthetic `[4628.67, 1983.02, 1899.98]` ms. The corresponding first device-initialization times were 7,388, 4,996 and 2,426 ms. Filesystem/driver caches were not cleared, so fresh process does not mean controlled cold cache; the reports do not establish the cause of these outliers.

AP242's reload median job is 11.3% faster on GPU, but once device initialization/cleanup are included across the actual three jobs, the difference is only 1.25% (210.20 versus 212.86 ms/job). This small unreplicated process-level difference is insufficient for a strong throughput claim. GPU reload initialization was approximately 46–48 ms and device cleanup 3.5–4.6 ms. The synthetic reload GPU process is 69.3% slower overall.

## What cached scenes and shared representation demonstrate

With the same prepared scene retained for three jobs, synthetic job medians are CPU 974.58 ms versus GPU 51.34 ms, and render-only medians 957.79 versus 34.50 ms. Including initial setup and teardown across that actual three-job process gives 1,007.60 versus 655.53 ms/job, a 35% elapsed-time reduction. AP242's retained three-job process similarly gives 150.11 versus 98.75 ms/job. Gear remains slower over the three-job process: 22.34 versus 43.03 ms/job. These runs repeat the same scene and render settings; they do not measure changed cameras, changed output dimensions, arbitrary new input files, or a long-running production daemon.

For synthetic new scenes, CPU fitting/packing/precision validation before GPU resource creation takes approximately 1,540 ms per upload. Resource creation adds approximately 154 ms; import approximately 39 ms. The 36 ms GPU render cannot compensate for this CPU preparation. Measured reload process CPU consumption per job is approximately 1,767 ms GPU versus 1,057 ms CPU. GPU execution is not included in CPU ticks, which have 10 ms granularity at 100 Hz; small differences or zero-length CPU substages must not be treated as precise savings.

AP242 reload process CPU consumption is approximately 203 versus 210 ms/job, only 20 ms difference across the entire three-job process (two clock ticks). Median fresh process CPU costs are also higher with GPU: gear 90 versus 20 ms, AP242 260 versus 210 ms, and synthetic 1,810 versus 1,050 ms. The large cached-render CPU savings therefore do not establish lower CPU consumption for fresh or arbitrary new-scene jobs.

The representation result is separate and useful: one source mesh has 6,146 positions and 12,288 triangles, placed 64 times. The compact scene's structural payload estimate is 353,400 bytes versus 62,914,560 bytes for 786,432 expanded triangles. Synthetic compact import is approximately 39–40 ms versus 90–93 ms for the legacy expanded import. However, shared-CPU then spends approximately 84 ms expanding again: reload process cost becomes 1,102.82 ms/job versus legacy CPU 1,069.15 ms/job, with nearly identical peak process RSS. Shared representation saves retained geometry payload; expanding it for the old CPU renderer removes most of that practical memory benefit. It is not proof that GPU arithmetic caused the import improvement.

A useful next memory experiment would let the CPU renderer consume instances directly, avoiding the expanded triangle vector and the GPU driver's baseline allocations. That is a design hypothesis, not an implemented or measured saving in this prototype. It would need its own correctness and complete-job memory comparison.

## Memory and conditions

| Input | CPU reload process HWM, MiB | GPU reload process HWM, MiB | Nominal explicit GPU bytes, MiB | CPU whole-case cgroup peak, MiB | GPU whole-case cgroup peak, MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Gear | 9.6 | 86.1 | 4.08 | 7.96 | 190.34 |
| AP242 | 36.9 | 114.9 | 4.55 | 52.74 | 226.55 |
| Controlled repeated assembly | 77.4 | 111.3 | 6.54 | 80.72 | 217.17 |

HWM is sampled process-lifetime resident high-water mark, not a per-job peak or complete system/GPU memory total. Cgroup peaks cover fresh, retained and reload together within each input/mode container. Driver allocations, mapped/shared pages, page cache and integrated-GPU memory have different and incomplete accounting scopes. RSS, cgroup charges and nominal explicit GPU allocation overlap: do not add them, or present the explicit GPU allocation budget as a total RAM cap. These observations show increased measured GPU process/cgroup memory in all three cases; they do not support a NAS memory-saving claim.

All nine cases have cgroup `failcnt=0`, the recorded memory and memory-plus-swap limits are both 512 MiB, and CPU affinity is CPU 3. The nominal explicit GPU allocation budget is 256 MiB. The host was already using swap and had approximately 883 MiB MemAvailable before the run; successful constrained tests are not measurements of unconstrained hardware throughput.

The final renderer bundle and test payloads were disk-backed at `/volume1/homes/max/.catwalk-gpu-20261018.n0HIq9/renderer` on Btrfs, as recorded in the [filesystem evidence](nas-results/bundle-filesystem-all.txt). However, three earlier CPU-baseline/probe directories remained in NAS `/tmp` during the entire matrix, contrary to the protocol's intended pre-run relocation. Their combined allocated size was approximately 100.1 MiB (102,504 KiB). They were moved, preserving contents, only after all measurements; the [relocation log](nas-post-run-scratch-relocation.log) and [script](nas-post-run-scratch-relocation.sh) record that cleanup. All modes ran with those earlier payloads present. This deviation remains part of the constrained host RAM/swap context; post-run cleanup is not a benchmark improvement, and available memory need not increase when disk page cache remains resident.

The [manifest](nas-results/manifest-all.json) records a 586,109-byte synthetic STEP input and a 2,120,559-byte real NIST AP242 input. The large expanded synthetic triangle payload results from controlled instancing, not a large production source file. It has no textures, materials, or authored normals and is explicitly not production-representative. Disk-backed deployment still permits page-cache memory use. Host memory context is preserved in the [before-run record](nas-results/host-before-all.txt).

The immediate requirement is to resolve the failed strict image qualification without loosening the frozen policy. Subsequent useful performance work is reducing validated per-instance CPU preparation and testing actual long-lived workloads, then repeating on the native Windows GPU. Preserve the current numerical fallback checks. Existing timings establish experimental behavior only; they do not certify correct production GPU thumbnails.
