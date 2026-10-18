# NAS CPU baseline protocol

The [profiler](../../../../../examples/profile_thumbnail.rs) measures Catwalk's existing CPU geometry-thumbnail path. [manifest.json](manifest.json) records the baseline source commit, exact profiler and executable hashes, input hashes, compiler, options and container restrictions. The binary uses the baseline code at commit `c06490333429ab85f9435ae8ca2df4b5a55ad60c`, before the concurrent GPU/shared-scene changes, plus the recorded profiler source.

All five fixtures completed on the NAS on 18 October 2026. Every fixture passed three fresh-process jobs and three retained-scene jobs within the unchanged timeout and memory restrictions. These are CPU reference measurements; no GPU renderer was involved.

The fixed workload is 512 × 512 pixels with one sample per axis. Each fixture runs three fresh child processes, followed by one import and three renders reusing the scene. Inputs cover the procedural gear, repeated STEP assembly, NIST AP242 mesh, JPEG-textured 3MF and glTF alpha modes. Their original repository paths are recorded in the manifest. No saved document previews are used.

[run-on-nas.sh](run-on-nas.sh) expects a temporary bundle containing `profile_thumbnail`, `inputs/`, and [run-one.sh](run-one.sh). Invoke it with `sh run-on-nas.sh /tmp/BUNDLE_DIRECTORY`. It uses the pre-existing Ubuntu 24.04 runtime image, CPU 3, a 512 MiB memory and memory-plus-swap ceiling, the host user's UID/GID, no GPU devices, no network, a read-only root filesystem, dropped capabilities and no new privileges. Each fixture has a 120-second timeout covering both modes. It writes only inside the temporary bundle and starts disposable containers; application containers are unchanged.

Raw reports are under [results/](results/). Each fixture has fresh-process and retained-scene JSON reports, exit statuses, stderr, libc identification and cgroup memory counters. [Host memory snapshots](results/host-before.txt) bracket the sequence. The NAS ran DSM 7.3.2 / Linux 4.4.302+; the runtime provided Ubuntu GLIBC 2.39. The deployed executable and all five input hashes match the manifest. Importer limits and validation were unchanged.

The following times are medians of three observations, in milliseconds. The total is measured by the parent from child-process launch through exit, including lifecycle and scheduling overhead. Its median is not the sum of independently calculated phase medians. Peak RSS is the maximum process-lifetime `VmHWM` across the three fresh children.

| Input | Triangles | Import + geometry ms | Render ms | PNG encoding ms | Fresh total ms | Fresh peak RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Gear OBJ | 512 | 0.6 | 14.9 | 7.5 | 25.7 | 7.9 |
| Repeated STEP assembly | 24 | 1.8 | 1.3 | 5.5 | 11.9 | 11.6 |
| AP242 tessellated STEP | 3,388 | 103.2 | 101.0 | 9.1 | 218.9 | 34.2 |
| JPEG-textured 3MF | 96,692 | 745.5 | 336.9 | 27.9 | 1,502.7 | 143.4 |
| glTF alpha modes | 61 | 155.2 | 123.0 | 17.9 | 302.8 | 52.9 |

The fresh-process total ranges were 25.1–48.0 ms, 11.8–67.1 ms, 218.9–291.7 ms, 1,123.0–2,281.1 ms and 300.8–393.5 ms respectively. The larger spread for the 3MF matters: three samples on the active NAS are a reference, not a stable throughput estimate. Background workloads and filesystem caches were not controlled. Before the run, the NAS reported about 745 MiB available RAM and 3.05 GiB occupied swap; those snapshots do not establish active swapping or attribute system changes to this test.

Retaining a scene removes its separately reported import from subsequent jobs; it does not substantially change rendering here. CPU values below include user and kernel time for the measured job, excluding one-time import in retained-scene mode. They use the observed 100 Hz process clock, so zero for the tiny STEP case is clock resolution, not zero CPU work.

| Input | Fresh job CPU ms, median | Retained-scene render ms, median | Retained-scene job CPU ms, median |
| --- | ---: | ---: | ---: |
| Gear OBJ | 20 | 14.0 | 20 |
| Repeated STEP assembly | 0 | 1.1 | 0 |
| AP242 tessellated STEP | 210 | 101.0 | 110 |
| JPEG-textured 3MF | 1,050 | 334.7 | 350 |
| glTF alpha modes | 290 | 123.1 | 140 |

Observed cgroup limits were 536,870,912 bytes for both memory and memory-plus-swap. All five `memory.failcnt` values were zero. Container-lifetime memory peaks, covering both modes, were 9.5, 9.4, 32.7, 141.4 and 52.4 MiB respectively. These counters and process RSS have different scopes; shared file pages can be charged outside this container, and kernel/GPU-driver coverage was not established. Linux 4.4's [memory-controller documentation](https://raw.githubusercontent.com/torvalds/linux/v4.4/Documentation/cgroups/memory.txt) describes shared-page charging and optional kernel accounting. Do not add these measurements together or treat either as complete NAS memory consumption.

The AP242, textured 3MF and alpha-mode fixtures have render phases large enough to make useful GPU comparison cases. Import, PNG encoding and lifecycle costs remain relevant to complete jobs. The 24-triangle assembly is a placement/control fixture; it cannot establish the benefit of retaining shared geometry in large assemblies. GPU execution and shared-scene savings still need separate, comparable measurements.

[summary.json](summary.json) contains the full numeric summary and raw-report hashes. Recreate it with `python3 summarize.py`; [summarize.py](summarize.py) verifies deployed hashes, exit statuses, iteration counts, dimensions, sampling and scene counts before calculating the summary. No native `catwalk` CLI timings are claimed: this is the geometry-only profiling example, with PNG encoded to memory and saved-preview dispatch bypassed.

Interpretation requires these distinctions:

- Import timing combines file reads, decoding, geometry processing, tessellation and validation. Those operations are not separately instrumented.
- Fresh-process wall time includes executable startup and report generation. It does not imply cold filesystem caches; neither caches nor production workloads are reset.
- Retained-scene jobs exclude the separately reported one-time import. Their render timings can be compared with fresh render phases, but their job times cannot establish an import-inclusive throughput improvement.
- RSS values are endpoint samples. `VmHWM` is a process-lifetime maximum, so reused-process values include setup and earlier jobs. CPU times use procfs clock ticks and short phases can round to zero.
- Cgroup peaks cover both modes and report memory charged to that container. They have a different scope from process RSS and do not establish coverage of all kernel/driver allocations. The container ceiling is a restriction, not a measured consumption figure.
- No GPU is involved in these baselines. They establish a CPU reference only; graphics-driver memory and GPU execution need comparable measurements later.
