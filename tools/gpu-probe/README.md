# Headless wgpu hardware capability probe

This isolated Rust program creates no window and no presentation surface. It enumerates adapters, saves hardware identity, exact backend, adapter limits and downlevel flags, then verifies offscreen graphics and a genuine compute dispatch. This is a capability smoke test, **not a representative Catwalk performance benchmark**.

See the [portable GPU acceleration goal](../../docs/gpu-acceleration-goal.md) for the broader architecture, validation targets and acceptance criteria. This standalone crate does not add a GPU backend or dependency to Catwalk itself. Run the following commands from this directory.

```sh
cargo build --release --locked
./target/release/catwalk-wgpu-probe \
  --backend all --output-dir results --iterations 16
```

For headless Linux EGL testing, use `EGL_PLATFORM=surfaceless` and `--backend gl`. Add `--require-intel` only when specifically validating an Intel adapter.

On native Windows, build with a native MSVC toolchain and run:

```powershell
cargo build --release --locked
.\target\release\catwalk-wgpu-probe.exe --backend dx12 --output-dir results --iterations 16
```

`--backend` accepts `all` (default), `gl`/`gles`, `dx12`, `vulkan`, and `metal`. This build enables DX12, Vulkan, and GLES; unavailable backends produce no eligible adapter and fail. Software/CPU devices, recognizable software renderer names, and unverified device types are rejected. `--require-intel` additionally restricts the hardware identity for the NAS test; omit it on other hardware. No override accepts a software adapter as successful hardware validation.

The current pinned wgpu 30.0.1 package could not resolve its Metal feature: crates.io offered `wgpu-core-deps-apple` only through 29.0.3 while this version requires ^30.0.0. Metal therefore remains uncompiled/unverified in this probe, without implying any architectural obstacle to a Metal renderer. The report always identifies the actual selected backend.

Outputs:

- `report.json`: written progressively so failures retain the adapter and stage details; includes all limits as a debug string and relevant limits as structured fields.
- `triangle.png`: 64×64 RGBA8 red triangle on blue. All pixels must be red or blue, its center red and its four corners blue, with an expected triangle coverage range.
- Console output: adapter JSON and a final pass/failure summary.

Compute reads 4096 vec4 values from one storage buffer, rotates/scales/translates them into another storage buffer, reads the result back and compares every component with an independent CPU expectation. Position and direction homogeneous coordinates are both included. No backend flag alone is reported as compute success. Missing compute flags or required storage/workgroup limits yields an explicit unsupported result after graphics validation.

Iteration zero is the first draw/dispatch after pipeline creation. Later iterations reuse the device, pipelines and buffers. Every invocation includes synchronization and readback, with CPU wall time separated into command encoding, submission/completion/readback and CPU verification. Initialization and PNG encoding are separate. Driver shader compilation can occur on the first invocation. These tiny fixed workloads do not establish application throughput, importer speed, or memory savings.

Exit status: `0` graphics and compute passed; `2` graphics passed but compute unsupported; `1` validation/runtime/hardware failure; `64` invalid command line; `124` global deadline. Each submitted GPU wait is bounded to five seconds; a watchdog terminates the entire process after `--deadline-seconds` (default 60, maximum 120). Driver entry points may block, so the independent watchdog bounds those too. The explicit GPU resources total 32 KiB graphics plus 192 KiB compute; driver memory and CPU overhead are extra and are not measured by this program.

Build natively for the host being tested. The reference Linux build used GNU x86_64 on Ubuntu 24.04 / glibc 2.39; that is one tested environment rather than a requirement for every platform. Linux runs dynamically load EGL/Vulkan libraries as needed and require the corresponding working hardware driver.

`run-container.sh` is an optional Linux/Intel validation helper for the NAS setup. It defaults to an existing `catwalk-gpu-probe:20261018` runtime image, an Intel-only GL probe, a 256 MiB container memory limit, no network and a read-only container filesystem. It does not build the image or install drivers. The tested Ubuntu 24.04 runtime provides `libegl1`, `libegl-mesa0`, `libgles2` and the working Intel Mesa/Amber driver; `/dev/dri/renderD128` is passed through with appropriate access. Override the documented environment variables in the script for binary, image, device and output paths. This helper does not modify Manyfold and is not needed for native Windows validation. A container memory limit does not establish a bound on all driver/GPU allocations or total system memory.

The included runtime `Dockerfile` records that image's recipe. To build a new runtime under a separate tag, run `docker build -t catwalk-wgpu-runtime .`, then `GPU_PROBE_IMAGE=catwalk-wgpu-runtime sh run-container.sh`. Build the Rust binary first and provide Docker access to the render node. Package versions follow the Ubuntu repository and are not pinned to the recorded probe image.

References used to implement the version-specific API:

- https://docs.rs/wgpu/30.0.1/wgpu/
- Local official crate source: `wgpu-30.0.1/src/api/{instance,device,adapter,render_pipeline,render_pass,compute_pipeline,buffer}.rs` and `wgpu-types-30.0.1/src/{instance,device,limits,adapter}.rs`.

Recorded run artifacts are kept separately from this source package. Negative cases should retain reports and process logs for forced software-renderer rejection, unavailable drivers/backends and invalid backend arguments.
