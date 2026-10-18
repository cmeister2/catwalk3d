# Portable GPU capability checks, 18 October 2026

These are hardware compatibility checks for the [GPU acceleration goal](../../../gpu-acceleration-goal.md), using the same [standalone Rust/wgpu probe](../../../../tools/gpu-probe/README.md). They do not measure Catwalk thumbnail performance or memory savings.

## Synology DS920+

- Intel Celeron J4125 / Intel UHD Graphics 600, DSM 7.3.2, Linux 4.4.302+ x86_64.
- wgpu 30.0.1 selected `Gl`, `IntegratedGpu`, vendor `0x8086`, renderer `Mesa DRI Intel(R) UHD Graphics 600 (GLK 2)`.
- Ubuntu 24.04 runtime image `catwalk-gpu-probe:20261018`, image ID `ffabbe46b4a5`; OpenGL 4.6 core profile, Mesa 21.3.9 Amber.
- Offscreen triangle pixels and all components of 4,096 transformed vectors passed. The initial invocation and 16 further invocations reused one device, pipelines and buffers.
- Isolated container: render-node passthrough, host user/render group, CPU 3 only, 256 MiB memory/swap ceiling, no network, read-only root filesystem, dropped capabilities and no new privileges. No application container was changed.
- The container ceiling is not a measurement or bound on all GPU/driver/system memory. The Intel GPU shares system RAM.

Results: [report.json](nas/report.json), [verified image](nas/triangle.png).

Build provenance: Linux x86_64 GNU, Rust 1.99.0; source SHA-256 `662658f68a0a1809314f4445d2664addcb4a1caede688ad1cce691cb1a393aad`, Cargo.lock SHA-256 `f54f00ca818408cec304208ecf28018661fba78c57af888de11ebce479e8f6fe`, binary SHA-256 `c7888619dd41a29f7047fcfbb2b36d77af4800de13ebcab768dff4ffa0e5765f`.

## Native Windows workstation

- AMD Radeon(TM) 780M, `IntegratedGpu`, PCI vendor `0x1002`, device `0x15BF`.
- The forced `--backend dx12` run selected `Dx12`, driver `32.0.31041.1004`.
- The default `--backend all` run selected `Vulkan`, `AMD proprietary driver`, driver information `26.8.1 (LLPC)`. Enumeration also found the Radeon DX12 adapter.
- Both runs passed the same offscreen triangle verification and all components of 4,096 transformed vectors, with one initial invocation and 16 further invocations reusing the device, pipelines and buffers. No window or presentation surface was created.
- Both reports reject `Microsoft Basic Render Driver` as a CPU/software adapter. The automatic run also excludes the Radeon GL adapter because its device type is reported as `Other`, which the probe cannot independently verify as hardware. This classification is not evidence that the GL driver cannot render.
- Built and executed as a native Windows program using MSVC Rust 1.95.0. WSL interop launched the executable; rendering used native Windows GPU drivers. These checks did not use or troubleshoot WSL2 graphics.

Results:

| Requested backends | Selected backend | Graphics / compute | Report | Verified image |
| --- | --- | --- | --- | --- |
| DX12 | DX12 | Passed / passed | [report.json](windows-dx12/report.json) | [triangle.png](windows-dx12/triangle.png) |
| All enabled backends | Vulkan | Passed / passed | [report.json](windows-auto/report.json) | [triangle.png](windows-auto/triangle.png) |

Build provenance: the source, Cargo.lock and embedded WGSL are identical to the NAS probe; source SHA-256 `662658f68a0a1809314f4445d2664addcb4a1caede688ad1cce691cb1a393aad`, Cargo.lock SHA-256 `f54f00ca818408cec304208ecf28018661fba78c57af888de11ebce479e8f6fe`. Both Windows runs used the same binary, SHA-256 `7d47f54c45857c25d95ebd14b6b2b1ae90cd742a50f5158d97eda21a2e86c219`.

Reproduction after a native Windows build:

```powershell
.\target\release\catwalk-wgpu-probe.exe --backend dx12 --output-dir windows-dx12 --iterations 16
.\target\release\catwalk-wgpu-probe.exe --backend all --output-dir windows-auto --iterations 16
```

## Interpretation

The same Rust source and WGSL workloads passed hardware graphics and compute through wgpu's GL backend on the NAS, and DX12 and Vulkan on native Windows. These results qualify those probe workloads on those configurations. They do not establish Vulkan support on the NAS, support for untested devices/backends, Catwalk renderer correctness for real models, acceleration, or reduced memory use. Recorded timings include synchronization and readback for tiny fixed workloads; they cannot substitute for complete thumbnail benchmarks.

Linux [negative controls](linux-negative-tests.json) also reject llvmpipe software rendering, a missing Vulkan driver and an unavailable backend. Further device/backend combinations remain untested.
