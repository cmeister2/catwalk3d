//! Compare legacy CPU, shared-import CPU and required-hardware GPU thumbnail costs.
//!
//! Build with `cargo build --release --features gpu --example profile_gpu`.
//! Run `profile_gpu MODEL --mode cpu|shared-cpu|gpu --report REPORT.json`.
//! Iterations reuse geometry and the GPU device; `--fresh-process` instead runs
//! one complete job per child, with independent process memory high-water marks.
//! `--reload-scene` imports and uploads anew for every job while reusing one device.

use catwalk3d::{
    Result, formats,
    gpu::{GpuOptions, GpuRenderer, GpuScene},
    instanced::InstancedScene,
    render::{self, RenderOptions},
    scene::{Scene, TriangleAppearance},
};
use clap::{Parser, ValueEnum};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    env, fs,
    io::Write,
    mem::size_of_val,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

/// Comparable geometry import and renderer selection.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    /// Import expanded triangles and retain the existing CPU scene.
    Cpu,
    /// Import shared meshes, expand for CPU rendering, then drop the shared scene.
    SharedCpu,
    /// Require a hardware adapter and retain uploaded GPU geometry only.
    Gpu,
}

impl Mode {
    /// Return the stable command-line and JSON representation.
    const fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::SharedCpu => "shared-cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// Native API selection, matching the main CLI's backend choices.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    /// Enumerate every compiled backend.
    Auto,
    /// Headless OpenGL/EGL.
    Gl,
    /// Windows DirectX 12.
    Dx12,
    /// Native Vulkan.
    Vulkan,
    /// Apple Metal.
    Metal,
}

impl Backend {
    /// Return the stable command-line and JSON representation.
    const fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gl => "gl",
            Self::Dx12 => "dx12",
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
        }
    }

    /// Restrict hardware enumeration to the requested native API.
    fn flags(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::all(),
            Self::Gl => wgpu::Backends::GL,
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Metal => wgpu::Backends::METAL,
        }
    }
}

/// Geometry-only profiling controls with explicit renderer selection.
#[derive(Parser)]
struct Args {
    /// Model geometry to import; saved document previews are bypassed.
    input: PathBuf,
    /// Import/renderer path to measure; GPU mode never falls back to CPU.
    #[arg(long, value_enum)]
    mode: Mode,
    /// Number of retained-scene renders, or independent children in fresh mode.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=1000))]
    iterations: u32,
    /// Final square thumbnail width and height.
    #[arg(long, default_value_t = 512, value_parser = clap::value_parser!(u32).range(1..=4096))]
    size: u32,
    /// Supersampling factor along each image axis.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=4))]
    samples: u32,
    /// JSON destination; omit to print JSON to stdout.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Optional PNG destination for the final measured render.
    #[arg(long)]
    png: Option<PathBuf>,
    /// Native API for GPU mode.
    #[arg(long, value_enum, default_value_t = Backend::Auto)]
    backend: Backend,
    /// Optional case-insensitive hardware adapter name substring.
    #[arg(long)]
    adapter: Option<String>,
    /// Explicit GPU scene allocation ceiling; excludes CPU and driver memory.
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u64).range(1..=8192))]
    gpu_memory_mib: u64,
    /// Launch a new process with one complete job for each requested iteration.
    #[arg(long, conflicts_with = "reload_scene")]
    fresh_process: bool,
    /// Reimport and prepare every job while retaining one GPU device and pipelines.
    #[arg(long)]
    reload_scene: bool,
}

/// Resource observations on Linux; unsupported systems retain null fields.
#[derive(Clone, Copy)]
struct Resources {
    /// Sampled resident set size in KiB.
    rss_kib: Option<u64>,
    /// Process-lifetime resident-set high-water mark in KiB.
    hwm_kib: Option<u64>,
    /// Cumulative user CPU clock ticks.
    user_ticks: Option<u64>,
    /// Cumulative kernel CPU clock ticks.
    system_ticks: Option<u64>,
}

/// Retained CPU or uploaded GPU scene, independent of the reusable GPU device.
enum Retained {
    /// Expanded geometry used by the unchanged software renderer.
    Cpu(Scene),
    /// Uploaded scene without a retained CPU source representation.
    Gpu(Box<GpuScene>),
}

/// Import, render, encode and report comparable phase measurements.
fn main() -> Result<()> {
    let entry = Instant::now();
    let args = Args::parse();
    let report = if args.fresh_process {
        fresh_processes(&args, entry)?
    } else {
        in_process(&args, entry)?
    };
    let mut encoded = serde_json::to_vec_pretty(&report)?;
    encoded.push(b'\n');
    if let Some(path) = &args.report {
        fs::write(path, encoded)?;
    } else {
        std::io::stdout().lock().write_all(&encoded)?;
    }
    Ok(())
}

/// Run one complete job per child without creating a device in the parent.
fn fresh_processes(args: &Args, entry: Instant) -> Result<Value> {
    let mut jobs = Vec::new();
    for iteration in 0..args.iterations {
        let mut command = Command::new(env::current_exe()?);
        command
            .arg(&args.input)
            .args(["--mode", args.mode.name(), "--iterations", "1"])
            .arg("--size")
            .arg(args.size.to_string())
            .arg("--samples")
            .arg(args.samples.to_string())
            .args(["--backend", args.backend.name()])
            .arg("--gpu-memory-mib")
            .arg(args.gpu_memory_mib.to_string());
        if let Some(adapter) = &args.adapter {
            command.arg("--adapter").arg(adapter);
        }
        if iteration + 1 == args.iterations
            && let Some(path) = &args.png
        {
            command.arg("--png").arg(path);
        }
        let start = Instant::now();
        let output = command.output()?;
        let wall = start.elapsed();
        if !output.status.success() {
            return Err(format!(
                "profile child failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        jobs.push(json!({
            "iteration": iteration,
            "subprocess_launch_to_exit_ms": milliseconds(wall),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "report": serde_json::from_slice::<Value>(&output.stdout)?,
        }));
    }
    Ok(json!({
        "schema_version": 1,
        "operation": "fresh_process_per_job",
        "reload_scene": false,
        "mode": args.mode.name(),
        "input": args.input.to_string_lossy(),
        "jobs": jobs,
        "parent_entry_to_report_ms": milliseconds(entry.elapsed()),
        "notes": [
            "Each child imports and renders once with an independent process VmHWM; the parent creates no GPU device.",
            "Launch-to-exit includes startup, accounting, device/scene teardown and JSON serialization; child phase totals have narrower scopes.",
            "Filesystem and driver caches are not cleared. Fresh process does not mean cold disk or shader caches.",
            "Use each child report for its memory/CPU observations; the parent does not measure child GPU/kernel/system memory."
        ],
    }))
}

/// Initialize a device once, then measure either retained-scene or reload-per-job work.
fn in_process(args: &Args, entry: Instant) -> Result<Value> {
    let ticks = clock_ticks_per_second();
    let work_start = Instant::now();
    let before = resources();
    let options = RenderOptions {
        width: args.size,
        height: args.size,
        supersampling: args.samples,
        ..RenderOptions::default()
    };
    let xy = matches!(formats::extension(&args.input)?.as_str(), "lbrn" | "lbrn2");
    if xy && matches!(args.mode, Mode::Gpu) {
        return Err("GPU profiling does not support LightBurn's dedicated XY view".into());
    }
    let (device, setup_device, adapter) = initialize_device(args, ticks)?;
    let (cached, setup) = if args.reload_scene {
        (None, Value::Null)
    } else {
        let (scene, phase) = prepare(args, &options, device.as_ref(), ticks)?;
        (Some(scene), phase)
    };
    let mut jobs = Vec::new();
    for iteration in 0..args.iterations {
        jobs.push(render_job(
            args,
            &options,
            device.as_ref(),
            cached.as_ref(),
            xy,
            iteration,
            ticks,
        )?);
    }
    let after_jobs = resources();
    let (_, cleanup) = measured(ticks, || {
        drop(cached);
        Ok(())
    })?;
    let (_, device_cleanup) = measured(ticks, || {
        drop(device);
        Ok(())
    })?;
    Ok(json!({
        "schema_version": 1,
        "operation": if args.reload_scene { "same_process_reuse_device_reload_scene" }
                     else { "same_process_reuse_scene_and_device" },
        "reload_scene": args.reload_scene,
        "mode": args.mode.name(),
        "input": args.input.to_string_lossy(),
        "width": args.size, "height": args.size, "supersampling": args.samples,
        "background_srgb": options.background, "fallback_color_srgb": options.color,
        "cpu_clock_ticks_per_second": ticks,
        "setup_device": setup_device,
        "adapter": adapter,
        "setup": setup,
        "jobs": jobs,
        "after_jobs_before_retained_cleanup": resource_values(after_jobs),
        "retained_cleanup": cleanup,
        "device_cleanup": device_cleanup,
        "process_entry_to_report_ms": milliseconds(entry.elapsed()),
        "setup_jobs_and_cleanup": phase(work_start.elapsed(), before, resources(), ticks),
        "notes": [
            "Device initialization is paid once in setup_device; the hardware adapter and actual renderer adapter/device capabilities are recorded separately. Adapter support can exceed enabled device limits/features.",
            "Default mode imports/prepares once in setup and all jobs reuse geometry. Warm rendering alone is not complete thumbnail throughput.",
            "Reload mode imports/prepares each job in scene_setup, drops that CPU/GPU scene before the next job, and reuses only the device/pipelines. Its job total includes import/upload/render/encoding/scene teardown; one-time device initialization is additional.",
            "Load includes reads, decoding, tessellation, geometry processing and validation; those importer internals are not separately instrumented.",
            "GPU upload includes CPU packing, fitting and resource creation; deferred upload/driver/shader work can appear in the first render.",
            "Upload substage walls separate CPU validation/fitting/packing from resource creation and scoped driver checks; resource creation does not guarantee DMA completion.",
            "CPU render includes validation/fitting/rasterization/resolve; GPU render includes submission/wait/resolve and final thumbnail readback only.",
            "CPU render substage walls separate global validation/normalization/camera fitting from canvas allocation, per-triangle projection/shading, rasterization and resolve. They do not split CPU-time tick deltas or measure GPU execution time.",
            "The shared CPU path drops the compact scene after expansion. The GPU path drops its CPU source scene before every measured render.",
            "PNG encoding targets memory. Optional final-file writing is separate and does not fsync.",
            "RSS values are endpoint samples, not phase peaks. VmHWM is cumulative per process and includes setup and previous iterations.",
            "CPU values use calibrated procfs tick deltas and exclude GPU execution; short intervals can round to zero. Non-Linux fields are null.",
            "Scene structural payload estimates exclude spare capacities, BTree nodes, allocator overhead and driver objects. Texture texels are counted once per distinct image.",
            "Explicit GPU bytes exclude driver allocations, transient copies, other GPU users and kernel/page-cache memory. Integrated GPU memory can overlap system RAM; do not add these estimates to RSS as total memory.",
            "Timers exclude boundary resource reads except enclosing job/setup/entry timers. Entry-to-report excludes executable startup and JSON serialization/writing, and includes retained-scene and device teardown.",
            "No filesystem/driver caches are cleared. This geometry-only harness bypasses saved previews and GPU mode never falls back to software."
        ],
    }))
}

/// Render one retained or newly loaded scene, then release all per-job allocations.
fn render_job(
    args: &Args,
    options: &RenderOptions,
    device: Option<&GpuRenderer>,
    cached: Option<&Retained>,
    xy: bool,
    iteration: u32,
    ticks: Option<u64>,
) -> Result<Value> {
    let job_entry = Instant::now();
    let job_before = resources();
    let (reloaded, setup) = if args.reload_scene {
        let (scene, phase) = prepare(args, options, device, ticks)?;
        (Some(scene), phase)
    } else {
        (None, Value::Null)
    };
    let scene = reloaded
        .as_ref()
        .or(cached)
        .ok_or("missing profiling scene")?;
    let ((image, cpu_timings), mut render_phase) = measured(ticks, || match scene {
        Retained::Cpu(scene) => {
            let (image, timings) = if xy {
                render::render_xy_profiled(scene, options)?
            } else {
                render::render_profiled(scene, options)?
            };
            Ok((image, Some(timings)))
        }
        Retained::Gpu(scene) => device
            .ok_or("missing profiling GPU device")?
            .render(scene)
            .map(|image| (image, None)),
    })?;
    render_phase["cpu_geometry_preparation_wall_ms"] =
        json!(cpu_timings.map(|timings| milliseconds(timings.geometry_preparation)));
    render_phase["cpu_rasterization_and_resolve_wall_ms"] =
        json!(cpu_timings.map(|timings| milliseconds(timings.rasterization_and_resolve)));
    let (encoded, encode_phase) = measured(ticks, || {
        let mut bytes = Vec::new();
        render::write_png(&image, &mut bytes)?;
        Ok(bytes)
    })?;
    let mut write_phase = Value::Null;
    if iteration + 1 == args.iterations
        && let Some(path) = &args.png
    {
        (_, write_phase) = measured(ticks, || Ok(fs::write(path, &encoded)?))?;
    }
    let png_bytes = encoded.len();
    std::hint::black_box(&encoded);
    drop(encoded);
    drop(image);
    let scene_cleanup = if args.reload_scene {
        let (_, phase) = measured(ticks, || {
            drop(reloaded);
            Ok(())
        })?;
        phase
    } else {
        Value::Null
    };
    Ok(json!({
        "iteration": iteration,
        "first_render": iteration == 0,
        "first_render_of_scene": args.reload_scene || iteration == 0,
        "scene_setup": setup,
        "render": render_phase,
        "png_encode_to_memory": encode_phase,
        "png_write": write_phase,
        "png_bytes": png_bytes,
        "scene_cleanup": scene_cleanup,
        "job_with_cleanup_and_accounting": phase(job_entry.elapsed(), job_before, resources(), ticks),
    }))
}

/// Create hardware pipelines once for all uploads, or return no device for CPU modes.
fn initialize_device(
    args: &Args,
    ticks: Option<u64>,
) -> Result<(Option<GpuRenderer>, Value, Value)> {
    if !matches!(args.mode, Mode::Gpu) {
        return Ok((None, Value::Null, Value::Null));
    }
    let (renderer, phase) = measured(ticks, || {
        GpuRenderer::new(GpuOptions {
            backends: args.backend.flags(),
            adapter_name: args.adapter.clone(),
            max_allocation_bytes: args.gpu_memory_mib * 1024 * 1024,
        })
    })?;
    let adapter = renderer.adapter_info();
    let capabilities = renderer.capabilities();
    let identity = json!({"name": adapter.name, "vendor": adapter.vendor,
    "device": adapter.device, "device_type": format!("{:?}", adapter.device_type),
    "backend": format!("{:?}", adapter.backend), "driver": adapter.driver,
    "driver_info": adapter.driver_info,
    "all_adapter_info_debug": format!("{adapter:#?}"),
    "capabilities": {
        "wgpu_version": "29.0.3",
        "source": "actual renderer adapter and successful device request; not the separate availability probe",
        "adapter_limits": limits_values(&capabilities.adapter_limits),
        "device_limits": limits_values(&capabilities.device_limits),
        "adapter_features_debug": format!("{:#?}", capabilities.adapter_features),
        "device_features_debug": format!("{:#?}", capabilities.device_features),
        "downlevel": {
            "all_debug": format!("{:#?}", capabilities.downlevel),
            "flags_debug": format!("{:?}", capabilities.downlevel.flags),
            "flags_bits": capabilities.downlevel.flags.bits(),
            "shader_model": format!("{:?}", capabilities.downlevel.shader_model),
            "compute_shaders_supported": capabilities.downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
        },
    }});
    Ok((Some(renderer), phase, identity))
}

/// Preserve every native limit in debug form and expose relevant limits as numeric JSON.
fn limits_values(limits: &wgpu::Limits) -> Value {
    json!({
        "all_limits_debug": format!("{limits:#?}"),
        "max_texture_dimension_1d": limits.max_texture_dimension_1d,
        "max_texture_dimension_2d": limits.max_texture_dimension_2d,
        "max_texture_dimension_3d": limits.max_texture_dimension_3d,
        "max_texture_array_layers": limits.max_texture_array_layers,
        "max_bind_groups": limits.max_bind_groups,
        "max_bindings_per_bind_group": limits.max_bindings_per_bind_group,
        "max_dynamic_uniform_buffers_per_pipeline_layout": limits.max_dynamic_uniform_buffers_per_pipeline_layout,
        "max_dynamic_storage_buffers_per_pipeline_layout": limits.max_dynamic_storage_buffers_per_pipeline_layout,
        "max_sampled_textures_per_shader_stage": limits.max_sampled_textures_per_shader_stage,
        "max_samplers_per_shader_stage": limits.max_samplers_per_shader_stage,
        "max_storage_buffers_per_shader_stage": limits.max_storage_buffers_per_shader_stage,
        "max_storage_textures_per_shader_stage": limits.max_storage_textures_per_shader_stage,
        "max_uniform_buffers_per_shader_stage": limits.max_uniform_buffers_per_shader_stage,
        "max_uniform_buffer_binding_size": limits.max_uniform_buffer_binding_size,
        "max_storage_buffer_binding_size": limits.max_storage_buffer_binding_size,
        "max_vertex_buffers": limits.max_vertex_buffers,
        "max_buffer_size": limits.max_buffer_size,
        "max_vertex_attributes": limits.max_vertex_attributes,
        "max_vertex_buffer_array_stride": limits.max_vertex_buffer_array_stride,
        "max_inter_stage_shader_variables": limits.max_inter_stage_shader_variables,
        "min_uniform_buffer_offset_alignment": limits.min_uniform_buffer_offset_alignment,
        "min_storage_buffer_offset_alignment": limits.min_storage_buffer_offset_alignment,
        "max_color_attachments": limits.max_color_attachments,
        "max_color_attachment_bytes_per_sample": limits.max_color_attachment_bytes_per_sample,
        "max_compute_workgroup_storage_size": limits.max_compute_workgroup_storage_size,
        "max_compute_invocations_per_workgroup": limits.max_compute_invocations_per_workgroup,
        "max_compute_workgroup_size_x": limits.max_compute_workgroup_size_x,
        "max_compute_workgroup_size_y": limits.max_compute_workgroup_size_y,
        "max_compute_workgroup_size_z": limits.max_compute_workgroup_size_z,
        "max_compute_workgroups_per_dimension": limits.max_compute_workgroups_per_dimension,
    })
}

/// Import one scene and release transitional representations before measured draws.
fn prepare(
    args: &Args,
    options: &RenderOptions,
    device: Option<&GpuRenderer>,
    ticks: Option<u64>,
) -> Result<(Retained, Value)> {
    let start = Instant::now();
    let before = resources();
    let mut setup = json!({"load_and_geometry": null,
        "expand_for_cpu": null, "gpu_prepare_and_upload": null,
        "cpu_geometry_preparation_wall_ms": null, "gpu_resource_creation_wall_ms": null,
        "drop_compact_source": null, "compact_scene": null, "expanded_scene": null,
        "explicit_gpu_allocation_bytes": null});
    let retained = match args.mode {
        Mode::Cpu => {
            let (scene, phase) = measured(ticks, || formats::load(&args.input))?;
            setup["load_and_geometry"] = phase;
            setup["expanded_scene"] = legacy_stats(&scene);
            Retained::Cpu(scene)
        }
        Mode::SharedCpu => {
            let (source, phase) = measured(ticks, || formats::load_instanced(&args.input))?;
            setup["load_and_geometry"] = phase;
            setup["compact_scene"] = shared_stats(&source)?;
            let (expanded, phase) = measured(ticks, || source.to_scene())?;
            setup["expand_for_cpu"] = phase;
            setup["expanded_scene"] = legacy_stats(&expanded);
            let (_, phase) = measured(ticks, || {
                drop(source);
                Ok(())
            })?;
            setup["drop_compact_source"] = phase;
            Retained::Cpu(expanded)
        }
        Mode::Gpu => {
            let renderer = device.ok_or("missing profiling GPU device")?;
            let (source, phase) = measured(ticks, || formats::load_instanced(&args.input))?;
            setup["load_and_geometry"] = phase;
            setup["compact_scene"] = shared_stats(&source)?;
            let (scene, phase) = measured(ticks, || renderer.upload(&source, options))?;
            setup["gpu_prepare_and_upload"] = phase;
            let timings = scene.upload_timings();
            setup["cpu_geometry_preparation_wall_ms"] =
                json!(milliseconds(timings.cpu_preparation));
            setup["gpu_resource_creation_wall_ms"] = json!(milliseconds(timings.resource_creation));
            setup["explicit_gpu_allocation_bytes"] = json!(scene.explicit_allocation_bytes());
            let (_, phase) = measured(ticks, || {
                drop(source);
                Ok(())
            })?;
            setup["drop_compact_source"] = phase;
            Retained::Gpu(Box::new(scene))
        }
    };
    setup["total_with_accounting"] = phase(start.elapsed(), before, resources(), ticks);
    Ok((retained, setup))
}

/// Count unique texture pixel storage referenced by the provided corner appearances.
fn texture_bytes<'a>(appearances: impl Iterator<Item = &'a TriangleAppearance>) -> usize {
    let mut images = HashSet::new();
    appearances
        .filter_map(|appearance| appearance.texture.as_ref())
        .filter_map(|texture| {
            images
                .insert(Arc::as_ptr(&texture.image))
                .then_some(texture.image.as_raw().len())
        })
        .sum()
}

/// Describe retained legacy primitives and inline structural payload bytes.
fn legacy_stats(scene: &Scene) -> Value {
    json!({"triangles": scene.triangles.len(), "points": scene.points.len(),
        "lines": scene.lines.len(), "triangle_appearances": scene.appearances.len(),
        "structural_payload_estimate_bytes": size_of_val(scene.triangles.as_slice())
            + size_of_val(scene.points.as_slice()) + size_of_val(scene.lines.as_slice())
            + scene.appearances.len() * std::mem::size_of::<(usize, TriangleAppearance)>(),
        "distinct_texture_texel_bytes": texture_bytes(scene.appearances.values())})
}

/// Describe compact source geometry before expansion or CPU-source release.
fn shared_stats(scene: &InstancedScene) -> Result<Value> {
    Ok(
        json!({"meshes": scene.meshes.len(), "instances": scene.instances.len(),
        "source_positions": scene.meshes.iter().map(|mesh| mesh.positions.len()).sum::<usize>(),
        "source_triangles": scene.meshes.iter().map(|mesh| mesh.faces.len()).sum::<usize>(),
        "logical_triangles": scene.expanded_triangle_count()?,
        "source_appearances": scene.meshes.iter().map(|mesh| mesh.appearances.len()).sum::<usize>(),
        "points": scene.points.len(), "lines": scene.lines.len(),
        "structural_payload_estimate_bytes": size_of_val(scene.meshes.as_slice())
            + size_of_val(scene.instances.as_slice()) + size_of_val(scene.points.as_slice())
            + size_of_val(scene.lines.as_slice()) + scene.meshes.iter().map(|mesh| {
                size_of_val(mesh.positions.as_slice()) + size_of_val(mesh.faces.as_slice())
                    + mesh.appearances.len() * std::mem::size_of::<(usize, TriangleAppearance)>()
            }).sum::<usize>(),
        "distinct_texture_texel_bytes": texture_bytes(scene.meshes.iter().flat_map(|mesh| mesh.appearances.values()))}),
    )
}

/// Observe one fallible operation without including resource reads in its wall timer.
fn measured<T>(ticks: Option<u64>, operation: impl FnOnce() -> Result<T>) -> Result<(T, Value)> {
    let before = resources();
    let start = Instant::now();
    let value = operation()?;
    let wall = start.elapsed();
    Ok((value, phase(wall, before, resources(), ticks)))
}

/// Convert a monotonic duration to fractional milliseconds.
fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Encode wall time, endpoint memory and process CPU deltas for one scope.
fn phase(wall: Duration, before: Resources, after: Resources, ticks: Option<u64>) -> Value {
    json!({"wall_ms": milliseconds(wall),
        "user_cpu_ms": cpu_delta(before.user_ticks, after.user_ticks, ticks),
        "system_cpu_ms": cpu_delta(before.system_ticks, after.system_ticks, ticks),
        "before": resource_values(before), "after": resource_values(after)})
}

/// Convert calibrated CPU clock-tick deltas without assuming a platform frequency.
#[allow(
    clippy::cast_precision_loss,
    reason = "Process clock ticks are approximate millisecond observations."
)]
fn cpu_delta(before: Option<u64>, after: Option<u64>, ticks: Option<u64>) -> Option<f64> {
    Some(after?.checked_sub(before?)? as f64 * 1000.0 / ticks? as f64)
}

/// Preserve unavailable resource fields as JSON null rather than zero measurements.
fn resource_values(value: Resources) -> Value {
    json!({"rss_kib": value.rss_kib, "process_lifetime_hwm_kib": value.hwm_kib,
        "user_cpu_ticks_total": value.user_ticks, "system_cpu_ticks_total": value.system_ticks})
}

/// Read Linux endpoint memory and cumulative CPU observations, or null elsewhere.
fn resources() -> Resources {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib = |name: &str| {
        status.lines().find_map(|line| {
            let mut fields = line.strip_prefix(name)?.split_whitespace();
            let value = fields.next()?.parse().ok()?;
            (fields.next()? == "kB").then_some(value)
        })
    };
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // comm can contain spaces and parentheses; state follows its final closing parenthesis.
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .map_or("", |(_, tail)| tail)
        .split_whitespace()
        .collect();
    Resources {
        rss_kib: kib("VmRSS:"),
        hwm_kib: kib("VmHWM:"),
        user_ticks: fields.get(11).and_then(|field| field.parse().ok()),
        system_ticks: fields.get(12).and_then(|field| field.parse().ok()),
    }
}

/// Query the operating system's procfs CPU tick frequency, leaving unsupported fields absent.
fn clock_ticks_per_second() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let output = Command::new("getconf").arg("CLK_TCK").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|rate| *rate > 0)
}
