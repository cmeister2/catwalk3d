# Actual-renderer NAS comparison and protocol

The completed NAS run contains **81 measured jobs and 27 validated experiment reports**, with source/binary identities frozen from a completed build. Keep CPU as the NAS default: complete fresh GPU jobs were slower for all three inputs, and measured process/cgroup memory was higher. Retaining uploaded scenes improved repeated rendering for the larger inputs; it did not establish a general new-thumbnail or memory benefit. The [measured recommendation](nas-recommendation.md) separates complete jobs, device reuse, representation effects and memory scopes.

**Strict image qualification failed:** all 12 supported CPU/GPU CLI pairs exceeded the unchanged interior maximum-error tolerance. All eight unsupported-material CPU/automatic-fallback pairs passed with identical PNG bytes. The smaller hardware test suite passed 18 image cases, but that does not override these larger 512px failures. Timings describe an experimental implementation, not production-qualified equivalent output. Preserve the failures under the original policy.

The [pixel investigation](pixel-investigation/diagnosis.md) attributes selected worst errors to subpixel triangle coverage at internal facet/occlusion edges. A sample can choose a different, sharply colored face even when average error and framing pass. Its independent geometry reconstruction explains the selected colors, including a synthetic case with a large depth separation. Exact hardware subpixel quantization was not measured. CPU coverage equivalence needs further design; these findings do not waive the original failures.

The [archived results](nas-results/) include every profiler report, command/exit log, output PNG, host/cgroup record, finalized manifest, checksum record, [profile summary](nas-results/profile-summary.json) and [image comparisons](nas-results/comparisons.json). The [completed Linux build metadata](builds/linux.json) identifies source checkpoint `6e00d13` and both binaries. The source-checkpoint image tests live separately under [nas-correctness-6e00d13](nas-correctness-6e00d13/). The protocol below explains reproduction; the [manifest template](manifest-template.json) deliberately remains unset rather than pretending to be a completed build.

The comparison uses the exact same GPU-enabled `profile_gpu` executable for `cpu`, `shared-cpu` and strict `gpu` modes. Every input/mode has three experiments, with three jobs each:

- Fresh processes: `--fresh-process` pays import and any GPU device initialization for every child, with independent process memory high-water marks.
- Cached scene/device: default mode imports/prepares once, then renders that retained scene three times.
- Reused device, reloaded scene: `--reload-scene` imports, expands or uploads, renders, encodes and drops a new scene for every job. GPU device/pipelines initialize once and are retained. CPU/shared-CPU also reload each job, giving a comparable complete-job control.

The inputs are the procedural gear, real NIST AP242 tessellated STEP and the synthetic repeated assembly. The latter has one source mesh with 12,288 triangles, 64 placements and 786,432 logical triangles; its generator manifest is packaged alongside the STEP file. It is a controlled representation workload, not a production-representative speedup claim.

Timing settings are fixed at 512 × 512, samples 1, CPU 3, 512 MiB memory and memory-plus-swap limits. Every mode gets identical render-node access and supplementary render group, host UID:GID, the existing Ubuntu runtime image, no network, read-only root filesystem, dropped capabilities, no new privileges and a 32 MiB temporary filesystem. No images are pulled and production containers are unchanged. Backend `gl` and adapter substring `UHD Graphics 600` are explicit. Strict GPU errors remain failures; CPU fallback is never counted as a GPU benchmark.

The [NAS runner](run-on-nas.sh) has a 120-second overall limit for each input/mode profile case, covering all three experiments. There are nine such cases, **81 measured jobs** in total. It records stdout, stderr, exit statuses, host memory snapshots, actual CPU affinity and available cgroup counters. Deployed binary/input/script/policy hashes, the finalized manifest and checksum-verification log are copied into results. Cases continue after failure; analysis retains those failures and the overall runner returns nonzero. Different stages can be run with `profile` or `cli`; existing results cannot be overwritten.

Source preparation, after the binaries are complete:

```sh
python3 prepare_bundle.py \
  --catwalk-binary /ACTUAL/BUILD/catwalk \
  --profile-binary /ACTUAL/BUILD/examples/profile_gpu \
  --build-metadata /LOCAL/COMPLETED-BUILD.json \
  --output /LOCAL/STAGING/NEW-RENDERER-BUNDLE
```

The supplied build metadata must contain `source_provenance`, `cargo_lock_sha256`, `compiler`, `command`, `catwalk_binary_sha256`, `profile_gpu_binary_sha256` and `profile_gpu_source_sha256`. Preserve the effective source snapshot/patch and untracked source hashes in `source_provenance`. [prepare_bundle.py](prepare_bundle.py) validates the supplied binary hashes and synthetic workload, then records input, policy and runner hashes and writes `SHA256SUMS`. It does not invent a source revision.

Stage the finalized bundle and native test payloads in a new **disk-backed NAS directory**, for example `/volume1/homes/max/.catwalk-gpu-20261018.XXXXXX`. NAS inspection found `/tmp` on a 1.9 GiB RAM-backed `tmpfs`, mounted `nosuid,nodev,noexec`; it held about 463 MiB at inspection, including an initial 257 MiB Catwalk debug-test payload. Relocate this investigation's earlier scratch payloads to the disk-backed staging area before taking the final host-memory baseline, preserving earlier evidence. The writable home under `/volume1/homes/max` is on the disk volume. Create a unique destination there, transfer the prepared bundle into it, then invoke:

```sh
nas_bundle=$(mktemp -d /volume1/homes/max/.catwalk-gpu-20261018.XXXXXX)
# Transfer the prepared bundle into "$nas_bundle" before running it.
sh "$nas_bundle/run-on-nas.sh" "$nas_bundle" all
```

The runner records `df` and the containing mount in `results/bundle-filesystem-{stage}.txt` and refuses known RAM-backed `tmpfs`/`ramfs` staging. This restriction concerns the NAS bundle and test payloads; the container's bounded 32 MiB `/tmp` remains part of the fixed runtime settings.

RAM-backed artifacts occupy host RAM or swap before a measured container starts and can change available memory, page-fault latency and pressure on other workloads. Their pages need not be charged to the reading container. Disk-backed files can still populate host page cache, and those charges are not fully attributable through process RSS or this container's counters. Preserve staging/mount and host-memory records; do not subtract bundle size from RSS or attribute a change caused by relocating artifacts to GPU execution. The earlier CPU reference used NAS `/tmp` staging, so its host-pressure context differs from these final comparisons.

Record every strict GPU report's actual hardware identity: Intel vendor `0x8086`, integrated hardware adapter, expected UHD Graphics 600 and `Gl` backend. The profiler records one-time device initialization in top-level `setup_device`, with identity in top-level `adapter`. Cached scene preparation is in `setup`; reload mode has `setup: null` and preparation/destruction in each job's `scene_setup`/`scene_cleanup`. Fresh children each use the cached schema with one job.

Scene preparation separates load/geometry, shared-to-expanded CPU conversion, GPU preparation/upload, and dropping transitional source geometry. GPU upload also reports nested CPU validation/fitting/packing and resource-creation wall times. CPU render reports nested global validation/normalization/camera-fit and rasterization/resolve walls. Those nested walls are not additional work to add to their enclosing upload/render phase, and are not separate CPU-tick or GPU-execution measurements. Deferred upload/driver work can appear in the first render. Load still combines decoding, tessellation and other importer geometry operations.

After fetching results, run the independent profile validator/summary:

```sh
python3 summary.py /LOCAL/RENDERER-BUNDLE
```

[summary.py](summary.py) validates deployed hashes against the finalized build/input/script/policy metadata, recorded memory limits/affinity, input/options/mode, current setup/render schema, triangle counts and every GPU adapter identity. It writes 27 experiment rows with every emitted ordered observation, raw-report hashes, failures, per-stage wall/CPU/RSS/HWM series, explicit GPU allocation estimates, first-render flags and min/median/max. Missing observations stay null or failed; they are never replaced with zero or fallback timing. A failed fresh wrapper can exit before emitting JSON, so earlier child measurements can be unavailable; their values are not reconstructed. Exit statuses and diagnostics remain recorded. A changed image policy fails provenance validation. Run with `--results` and `--manifest` to analyze archived evidence whose paths differ from the original bundle.

Fresh launch-to-exit, cached frame time and complete reload-job time have different scopes. The summary also reports each process's measured setup/jobs/cleanup interval and its finite three-job amortization, including one device initialization; that scope excludes executable startup and JSON writing. CPU observations use calibrated procfs ticks: 100 Hz means 10 ms resolution and short stages can round to zero. Cached scene setup is counted once, fresh setup three times, and reload setup three times. Case-wide cgroup peaks cover all experiments and cannot be assigned to any single one. Nonzero memory-limit counters remain visible as constrained-run evidence.

Legacy CPU versus shared-import CPU separates importer representation/conversion costs. Shared-import CPU versus GPU still differs in CPU expansion versus GPU upload and retained representation; it does not isolate GPU arithmetic alone. Use the separate stage and structural-payload observations to explain those differences. Rendering an already cached scene cannot establish complete new-thumbnail throughput; the reload experiment is required for reused-device job throughput.

The native CLI correctness stage is separate from timing. Gear, AP242 and the synthetic assembly each render with CPU and strict GPU at samples 1 and 2, with opaque and transparent backgrounds: 24 PNGs. JPEG-textured 3MF and glTF alpha-mode inputs each render with CPU and `auto` at the same four settings: 16 PNGs. Their eight strict GPU commands must fail with the explicit unsupported-material diagnostic and no output PNG. A timeout, OOM or unrelated initialization error does not satisfy that rejection. Each CLI invocation has a 120-second limit; the full matrix for one input is bounded at 600 seconds. `--render-geometry` bypasses saved previews.

Download `results/`, then run locally:

```sh
python3 compare_results.py /LOCAL/RENDERER-BUNDLE/results
```

The comparator requires **Pillow**, already available here as version 10.2.0; it otherwise uses Python's standard library. No Python/Pillow installation is needed inside the NAS image. [compare_results.py](compare_results.py) checks command expectations and writes all 20 required PNG comparisons, including failures and missing images. [compare_png.py](compare_png.py) can also compare a single pair; add `--transparent` for transparent-background coverage, or `--exact` for fallback identity.

[policy.json](policy.json) records the proposed tolerances **before actual hardware results**:

- Images must have identical dimensions and original coordinates. No fitting, translation, resizing, registration or crop is permitted.
- Foreground coverage uses nonzero alpha for transparent backgrounds. For the default opaque background, coverage differs from RGB `(245,247,250)` by more than two byte values or has non-opaque alpha.
- Coverage IoU must be at least 0.995, area error at most 0.5%, bounding-coordinate error at most one pixel and centroid drift at most 0.25 pixels. The centroid gate rejects a uniform one-pixel shift even when its IoU and bounds pass.
- Interior error uses the common foreground eroded by one pixel in all directions. At least 32 interior pixels and 25% of reference foreground must remain; an empty/thin mask cannot pass by hiding all errors. Internal shading/visibility edges are not excluded.
- Interior straight-RGBA errors must have maximum at most 3 and mean at most 0.5 byte values per channel. Background pixels beyond the one-pixel coverage edge must differ by at most 1. Global and per-channel RGBA maximum, mean, RMSE and changed-pixel counts are always reported.
- CPU/automatic-fallback PNGs must have identical decoded RGBA **and identical encoded PNG bytes**. Fallback does not use the GPU tolerance policy.

If a gate fails, retain the failure and inspect geometry, materials, visibility and numerical behavior. Any later policy revision must be explicit, justified and reported alongside results under the original policy; do not silently loosen thresholds after seeing the images. PNG comparisons cannot establish correctness for invisible geometry, symmetric orientation errors or every topology/material case; importer and renderer tests remain necessary.

Memory interpretation follows the [earlier CPU baseline](../cpu-baseline/README.md). Endpoint RSS is not a phase peak; `VmHWM` accumulates across setup and retained jobs. Cgroup counters measure charged memory and can miss shared pages or driver allocations. Explicit GPU allocation estimates are not total GPU/system memory, and Intel graphics shares RAM. Preserve these scopes rather than summing overlapping counters. Shader/filesystem caches and active NAS workloads are not reset; keep every individual observation, including first renders and variability.
