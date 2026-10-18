# Native Windows renderer matrix

These are prepared scripts, not measured results. Do not execute them until the build owner supplies the completed Windows `catwalk.exe`, `profile_gpu.exe` and matching source/build/hash metadata. No Windows or WSL GPU work was run while preparing this directory. Existing NAS protocol, library sources and the frozen PNG policy are unchanged.

[run_windows.py](run_windows.py) runs native Windows executables through Windows PowerShell `Start-Process`. Executables and all file arguments use WSL UNC paths; no graphics API runs in WSL. Native stdout, stderr, PID, exit code and wall time are recorded per command. Python also records the separate outer interop/PowerShell wall; use internal profiler scopes for renderer comparisons, not this orchestration overhead. A native 120-second deadline kills that command's Windows process tree. If the outer launcher itself times out, the suite stops and retains the PID file for inspection rather than starting overlapping work.

The completed build metadata has the same required fields as the NAS bundle: `source_provenance`, `compiler`, `command`, `cargo_lock_sha256`, `profile_gpu_source_sha256`, `catwalk_binary_sha256` and `profile_gpu_binary_sha256`. The script verifies the supplied hashes and PE signatures, copies immutable executables/inputs into a new output directory, and records their hashes. It never derives source provenance from a worktree that may have changed since compilation.

```sh
python3 run_windows.py \
  --catwalk-binary /ACTUAL/WINDOWS/BUILD/catwalk.exe \
  --profile-binary /ACTUAL/WINDOWS/BUILD/examples/profile_gpu.exe \
  --build-metadata /tmp/COMPLETED-WINDOWS-BUILD.json \
  --output /tmp/NEW-WINDOWS-RENDERER-RUN
python3 analyze.py /tmp/NEW-WINDOWS-RENDERER-RUN
```

`--prepare-only` stages a supplied completed build without invoking Windows. Every invocation requires a new output directory. `--stage profile` or `--stage cli` can narrow a diagnostic run; the full analysis expects the complete default matrix.

Each of **DX12 and Vulkan** runs gear, real AP242 STEP and the controlled synthetic repeated assembly, with modes `cpu`, `shared-cpu` and strict `gpu`, and experiments fresh-process, cached-scene/device and reused-device/reloaded-scene. Each has three jobs at 512px/samples1: **162 measured jobs** across 54 experiment reports. Every GPU request selects `Radeon(TM) 780M`. [analyze.py](analyze.py) requires AMD vendor `0x1002`, device `0x15bf`, integrated hardware and the requested native backend. It preserves each actual renderer's complete `adapter.capabilities` object, including adapter/device limits and features from wgpu 29.0.3. No separate availability probe substitutes for those capabilities.

The current profiler's Windows process CPU, RSS and HWM fields are **null/unavailable**, not zero. This runner does not invent replacements. Explicit GPU allocation estimates exclude drivers and shared/system RAM; they cannot establish total memory consumption. Windows runs use the host's CPU/memory policy, without NAS CPU3 pinning or cgroups, so they are not a controlled NAS-versus-Windows performance comparison. Video-controller inventory, when collected, is driver metadata rather than allocation evidence.

Correctness uses the same native CLI matrix as the NAS protocol for each backend: three supported inputs with CPU/GPU at samples1/2 and opaque/transparent backgrounds, plus textured 3MF and alpha-mode glTF CPU/automatic-fallback pairs and expected strict-GPU material rejection. Profiling does not write PNGs, avoiding a final-job file-write bias; CLI PNG generation is separate. Analysis reuses the unchanged [PNG comparator and policy](../README.md), requiring 20 comparisons per backend, including exact encoded/decoded fallback identity. Missing commands/images, wrong hardware, unsupported strict GPU work and tolerance failures remain failures.

Shader/filesystem caches and Windows background workloads are not reset. Fresh native child lifetime, cached frame latency and complete reload-job latency remain different scopes. Individual observations and min/median/max series are retained. A failed fresh wrapper can omit earlier un-emitted child observations; those remain missing rather than reconstructed. Native physical unit tests are a separate validation step and are not replaced by this benchmark script.
