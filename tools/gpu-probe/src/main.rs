//! Bounded, offscreen portable wgpu availability probe; never accepts software as hardware.
use serde_json::{Value, json};
use std::{
    error::Error,
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SIDE: u32 = 64;
const ELEMENTS: usize = 4096;
const WORKGROUP: u32 = 64;
const COMPUTE_BYTES: u64 = ELEMENTS as u64 * 16;
const WAIT: Duration = Duration::from_secs(5);

const RENDER_SHADER: &str = r#"
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-0.8, -0.8), vec2<f32>(0.8, -0.8), vec2<f32>(0.0, 0.8));
    return vec4<f32>(positions[vertex], 0.0, 1.0);
}
@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
"#;
const COMPUTE_SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> input_positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> output_positions: array<vec4<f32>>;
@compute @workgroup_size(64) fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= arrayLength(&input_positions)) { return; }
    let p = input_positions[i];
    // Rotation, non-uniform scale, translation; homogeneous w is preserved.
    output_positions[i] = vec4<f32>(-2.0 * p.y + 3.0 * p.w, 0.5 * p.x - 4.0 * p.w, p.z + 5.0 * p.w, p.w);
}
"#;

#[derive(Clone)]
struct Reporter {
    path: PathBuf,
    state: Arc<Mutex<Value>>,
}
impl Reporter {
    fn set(&self, key: &str, value: Value) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state[key] = value;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&*state)?)?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn hardware_reason(
    info: &wgpu::AdapterInfo,
    require_intel: bool,
) -> std::result::Result<(), String> {
    let identity = format!("{} {} {}", info.name, info.driver, info.driver_info).to_lowercase();
    if info.device_type == wgpu::DeviceType::Cpu
        || [
            "llvmpipe",
            "softpipe",
            "swrast",
            "swiftshader",
            "software",
            "lavapipe",
        ]
        .iter()
        .any(|s| identity.contains(s))
    {
        return Err(format!("software/CPU adapter rejected: {}", info.name));
    }
    if !matches!(
        info.device_type,
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::DiscreteGpu
    ) {
        return Err(format!(
            "unverified hardware device type {:?}",
            info.device_type
        ));
    }
    if require_intel && info.vendor != 0x8086 && !identity.contains("intel") {
        return Err(format!(
            "expected NAS Intel GPU, found {} (vendor {:#x})",
            info.name, info.vendor
        ));
    }
    Ok(())
}
fn limits_json(l: &wgpu::Limits) -> Value {
    json!({
        "all_limits_debug": format!("{l:#?}"),
        "max_texture_dimension_2d": l.max_texture_dimension_2d,
        "max_buffer_size": l.max_buffer_size,
        "max_bind_groups": l.max_bind_groups,
        "max_vertex_buffers": l.max_vertex_buffers,
        "max_vertex_attributes": l.max_vertex_attributes,
        "max_storage_buffers_per_shader_stage": l.max_storage_buffers_per_shader_stage,
        "max_storage_textures_per_shader_stage": l.max_storage_textures_per_shader_stage,
        "max_storage_buffer_binding_size": l.max_storage_buffer_binding_size,
        "max_compute_workgroup_storage_size": l.max_compute_workgroup_storage_size,
        "max_compute_invocations_per_workgroup": l.max_compute_invocations_per_workgroup,
        "max_compute_workgroup_size_x": l.max_compute_workgroup_size_x,
        "max_compute_workgroup_size_y": l.max_compute_workgroup_size_y,
        "max_compute_workgroup_size_z": l.max_compute_workgroup_size_z,
        "max_compute_workgroups_per_dimension": l.max_compute_workgroups_per_dimension,
        "min_uniform_buffer_offset_alignment": l.min_uniform_buffer_offset_alignment,
        "min_storage_buffer_offset_alignment": l.min_storage_buffer_offset_alignment,
    })
}
fn adapter_json(a: &wgpu::Adapter, require_intel: bool) -> Value {
    let i = a.get_info();
    let d = a.get_downlevel_capabilities();
    json!({"name": i.name, "vendor": i.vendor, "device": i.device, "device_type": format!("{:?}", i.device_type), "backend": format!("{:?}", i.backend), "driver": i.driver, "driver_info": i.driver_info,
        "hardware_validation": hardware_reason(&i, require_intel).err().unwrap_or_else(|| "accepted hardware".to_string()),
        "limits": limits_json(&a.limits()), "downlevel_flags": format!("{:?}", d.flags), "shader_model": format!("{:?}", d.shader_model), "features": format!("{:?}", a.features()),
        "compute_flag": d.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)})
}
fn map_read(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
) -> Result<Vec<u8>> {
    let (tx, rx) = mpsc::sync_channel(1);
    buffer.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(WAIT),
    })?;
    rx.recv_timeout(WAIT)??;
    let view = buffer.get_mapped_range(..)?;
    let bytes = view.to_vec();
    drop(view);
    buffer.unmap();
    Ok(bytes)
}
fn validation(scope: wgpu::ErrorScopeGuard) -> Result<()> {
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(format!("wgpu validation error: {e}").into());
    }
    Ok(())
}
struct Graphics {
    pipeline: wgpu::RenderPipeline,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}
impl Graphics {
    fn new(device: &wgpu::Device) -> Result<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("triangle WGSL"),
            source: wgpu::ShaderSource::Wgsl(RENDER_SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("headless triangle"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("64x64 RGBA8 offscreen"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle readback"),
            size: u64::from(SIDE * SIDE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        validation(scope)?;
        Ok(Self {
            pipeline,
            target,
            view,
            readback,
        })
    }
    fn draw(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(Vec<u8>, Value)> {
        let start = Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("triangle encoder"),
        });
        {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("triangle pass"),
                color_attachments: &colors,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIDE * 4),
                    rows_per_image: Some(SIDE),
                },
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        let encode_ms = ms(start);
        let submit_started = Instant::now();
        let submission = queue.submit([encoder.finish()]);
        let rgba = map_read(device, &self.readback, submission)?;
        let submit_readback_ms = ms(submit_started);
        validation(scope)?;
        let check_start = Instant::now();
        let pixel = |x: u32, y: u32| -> &[u8] {
            let offset = ((y * SIDE + x) * 4) as usize;
            &rgba[offset..offset + 4]
        };
        let red = |p: &[u8]| p[0] > 240 && p[1] < 15 && p[2] < 15 && p[3] > 240;
        let blue = |p: &[u8]| p[0] < 15 && p[1] < 15 && p[2] > 240 && p[3] > 240;
        let red_count = rgba.as_chunks::<4>().0.iter().filter(|&p| red(p)).count();
        if !red(pixel(32, 32))
            || ![(0, 0), (63, 0), (0, 63), (63, 63)]
                .iter()
                .all(|&(x, y)| blue(pixel(x, y)))
            || !(1000..1600).contains(&red_count)
            || !rgba.as_chunks::<4>().0.iter().all(|p| red(p) || blue(p))
        {
            return Err(format!(
                "triangle verification failed: center={:?}, corner={:?}, red_count={red_count}",
                pixel(32, 32),
                pixel(0, 0)
            )
            .into());
        }
        let verification_ms = ms(check_start);
        let timing = json!({"encode_commands_ms": encode_ms, "submit_complete_readback_ms": submit_readback_ms, "cpu_verification_ms": verification_ms, "total_ms": ms(start), "red_pixels": red_count, "center_rgba": pixel(32,32), "corner_rgba": pixel(0,0)});
        Ok((rgba, timing))
    }
}
struct Compute {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    input: Vec<[f32; 4]>,
}
fn transformed(p: [f32; 4]) -> [f32; 4] {
    [
        -2.0 * p[1] + 3.0 * p[3],
        0.5 * p[0] - 4.0 * p[3],
        p[2] + 5.0 * p[3],
        p[3],
    ]
}
impl Compute {
    fn new(device: &wgpu::Device) -> Result<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let input: Vec<[f32; 4]> = (0..ELEMENTS)
            .map(|i| {
                [
                    (i % 257) as f32 * 0.125 - 16.0,
                    (i % 127) as f32 * 0.25 - 8.0,
                    (i % 61) as f32 * 0.5 - 4.0,
                    if i % 2 == 0 { 1.0 } else { 0.0 },
                ]
            })
            .collect();
        let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compute input positions"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compute output positions"),
            size: COMPUTE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compute readback"),
            size: COMPUTE_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("position transform WGSL"),
            source: wgpu::ShaderSource::Wgsl(COMPUTE_SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("position transforms"),
            layout: None,
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("position buffers"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        validation(scope)?;
        Ok(Self {
            pipeline,
            bind,
            output,
            readback,
            input,
        })
    }
    fn dispatch(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Value> {
        let start = Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("compute encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("position transform pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.dispatch_workgroups((ELEMENTS as u32).div_ceil(WORKGROUP), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.output, 0, &self.readback, 0, COMPUTE_BYTES);
        let encode_ms = ms(start);
        let submit_start = Instant::now();
        let submission = queue.submit([encoder.finish()]);
        let bytes = map_read(device, &self.readback, submission)?;
        let submit_readback_ms = ms(submit_start);
        validation(scope)?;
        let check_start = Instant::now();
        let mut max_error = 0.0_f32;
        for (i, chunk) in bytes.as_chunks::<16>().0.iter().enumerate() {
            let expected = transformed(self.input[i]);
            for j in 0..4 {
                let got = f32::from_ne_bytes(chunk[j * 4..j * 4 + 4].try_into().unwrap());
                let delta = (got - expected[j]).abs();
                if !got.is_finite() || delta > 1e-6 {
                    return Err(format!(
                        "compute verification failed at element {i}, component {j}: {got} != {}",
                        expected[j]
                    )
                    .into());
                }
                max_error = max_error.max(delta);
            }
        }
        Ok(
            json!({"encode_commands_ms": encode_ms, "submit_complete_readback_ms": submit_readback_ms, "cpu_verification_ms": ms(check_start), "total_ms": ms(start), "checked_vec4s": ELEMENTS, "max_abs_error": max_error}),
        )
    }
}
fn write_png(path: &Path, rgba: &[u8]) -> Result<()> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), SIDE, SIDE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(())
}
fn run(
    report: &Reporter,
    output: &Path,
    iterations: usize,
    backends: wgpu::Backends,
    require_intel: bool,
) -> Result<bool> {
    let start = Instant::now();
    report.set("stage", json!("enumerating_adapters"))?;
    let instance_start = Instant::now();
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    let instance = wgpu::Instance::new(descriptor);
    let adapters = pollster::block_on(instance.enumerate_adapters(backends));
    let instance_enumeration_ms = ms(instance_start);
    let descriptions: Vec<_> = adapters
        .iter()
        .map(|a| adapter_json(a, require_intel))
        .collect();
    report.set("adapters", json!(descriptions))?;
    for a in &adapters {
        println!(
            "Adapter: {}",
            serde_json::to_string(&adapter_json(a, require_intel))?
        );
    }
    let adapter = adapters.into_iter().find(|a| hardware_reason(&a.get_info(), require_intel).is_ok()).ok_or("No eligible hardware adapter: software/CPU, unknown, or mismatched adapters are not accepted")?;
    report.set("selected_adapter", adapter_json(&adapter, require_intel))?;
    report.set("hardware_validation", json!("passed"))?;
    let limits = adapter.limits();
    let compute_supported = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && limits.max_storage_buffers_per_shader_stage >= 2
        && limits.max_storage_buffer_binding_size >= COMPUTE_BYTES
        && limits.max_compute_invocations_per_workgroup >= WORKGROUP
        && limits.max_compute_workgroup_size_x >= WORKGROUP
        && limits.max_compute_workgroups_per_dimension >= (ELEMENTS as u32).div_ceil(WORKGROUP);
    report.set("compute", json!({"status": if compute_supported {"available_not_yet_tested"} else {"unsupported"}, "required": {"storage_buffers": 2, "bytes_per_buffer": COMPUTE_BYTES, "workgroup_size_x": WORKGROUP, "workgroups": (ELEMENTS as u32).div_ceil(WORKGROUP)}, "reason": if compute_supported {"flag and limits meet test requirements"} else {"compute downlevel flag or required storage/workgroup limits absent"}}))?;
    let mut requested = wgpu::Limits::downlevel_webgl2_defaults().using_alignment(limits.clone());
    if compute_supported {
        requested.max_storage_buffers_per_shader_stage = 2;
        requested.max_storage_buffer_binding_size = COMPUTE_BYTES;
        requested.max_compute_invocations_per_workgroup = WORKGROUP;
        requested.max_compute_workgroup_size_x = WORKGROUP;
        requested.max_compute_workgroup_size_y = 1;
        requested.max_compute_workgroup_size_z = 1;
        requested.max_compute_workgroups_per_dimension = (ELEMENTS as u32).div_ceil(WORKGROUP);
    }
    report.set("requested_limits", limits_json(&requested))?;
    report.set("stage", json!("creating_device"))?;
    let device_start = Instant::now();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Catwalk NAS wgpu smoke probe"),
        required_features: wgpu::Features::empty(),
        required_limits: requested,
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))?;
    let device_create_ms = ms(device_start);
    report.set("stage", json!("creating_graphics_pipeline"))?;
    let graphics_start = Instant::now();
    let graphics = Graphics::new(&device)?;
    let graphics_pipeline_resources_ms = ms(graphics_start);
    let mut timing = json!({"instance_and_enumeration_ms": instance_enumeration_ms, "device_creation_ms": device_create_ms, "graphics_pipeline_resources_ms": graphics_pipeline_resources_ms});
    let mut draw_times = Vec::new();
    for iteration in 0..=iterations {
        report.set("stage", json!(format!("graphics_iteration_{iteration}")))?;
        let (rgba, mut frame) = graphics.draw(&device, &queue)?;
        frame["iteration"] = json!(iteration);
        frame["device_reused"] = json!(iteration > 0);
        draw_times.push(frame);
        if iteration == 0 {
            let png_start = Instant::now();
            write_png(&output.join("triangle.png"), &rgba)?;
            timing["png_encode_ms"] = json!(ms(png_start));
            report.set("graphics", json!({"status": "passed", "width": SIDE, "height": SIDE, "format": "Rgba8Unorm", "pixels_verified": SIDE*SIDE, "png": "triangle.png"}))?;
        }
    }
    timing["draw_iterations"] = json!(draw_times);
    report.set("timings", timing.clone())?;
    if compute_supported {
        report.set("stage", json!("creating_compute_pipeline"))?;
        let compute_start = Instant::now();
        let compute = Compute::new(&device)?;
        timing["compute_pipeline_resources_ms"] = json!(ms(compute_start));
        let mut compute_times = Vec::new();
        for iteration in 0..=iterations {
            report.set("stage", json!(format!("compute_iteration_{iteration}")))?;
            let mut t = compute.dispatch(&device, &queue)?;
            t["iteration"] = json!(iteration);
            t["device_reused"] = json!(iteration > 0);
            compute_times.push(t);
        }
        timing["compute_iterations"] = json!(compute_times);
        report.set("compute", json!({"status": "passed", "checked_vec4s_per_dispatch": ELEMENTS, "dispatches": iterations + 1, "operation": "rotate + nonuniform scale + homogeneous translation", "tolerance": 1e-6}))?;
    }
    timing["total_probe_ms"] = json!(ms(start));
    report.set("timings", timing)?;
    report.set("stage", json!("finished"))?;
    report.set(
        "status",
        json!(if compute_supported {
            "passed"
        } else {
            "graphics_passed_compute_unsupported"
        }),
    )?;
    println!(
        "{}: verified hardware graphics{}; report {}",
        if compute_supported { "PASS" } else { "PARTIAL" },
        if compute_supported {
            " and compute"
        } else {
            "; compute unsupported"
        },
        report.path.display()
    );
    Ok(compute_supported)
}
fn main() {
    let mut output = PathBuf::from(".");
    let mut iterations = 16;
    let mut deadline_seconds = 60;
    let mut backends = wgpu::Backends::all();
    let mut require_intel = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--require-intel" {
            require_intel = true;
            continue;
        }
        let value = match arg.as_str() {
            "--output-dir" | "--output" | "--backend" | "--iterations" | "--deadline-seconds" => {
                args.next().unwrap_or_else(|| {
                    eprintln!("{arg} requires a value");
                    std::process::exit(64)
                })
            }
            "--help" => {
                println!(
                    "catwalk-wgpu-probe [--backend gl|dx12|vulkan|metal|all] [--require-intel] [--output-dir DIR] [--iterations 0..1000] [--deadline-seconds 1..120]\nRequires identified GPU hardware; software adapters always rejected. --require-intel additionally restricts vendor (NAS). Exit 0: graphics+compute passed; 2: graphics passed, compute unsupported; 1: failed. Not a representative performance benchmark."
                );
                return;
            }
            _ => {
                eprintln!("unknown argument {arg}");
                std::process::exit(64)
            }
        };
        match arg.as_str() {
            "--output-dir" | "--output" => output = value.into(),
            "--backend" => {
                backends = match value.as_str() {
                    "gl" | "gles" => wgpu::Backends::GL,
                    "dx12" => wgpu::Backends::DX12,
                    "vulkan" => wgpu::Backends::VULKAN,
                    "metal" => wgpu::Backends::METAL,
                    "all" => wgpu::Backends::all(),
                    _ => {
                        eprintln!("invalid backend");
                        std::process::exit(64)
                    }
                }
            }
            "--iterations" => {
                iterations = value
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n <= 1000)
                    .unwrap_or_else(|| {
                        eprintln!("invalid iterations");
                        std::process::exit(64)
                    })
            }
            _ => {
                deadline_seconds = value
                    .parse::<u64>()
                    .ok()
                    .filter(|n| (1..=120).contains(n))
                    .unwrap_or_else(|| {
                        eprintln!("invalid deadline");
                        std::process::exit(64)
                    })
            }
        }
    }
    if let Err(e) = fs::create_dir_all(&output) {
        eprintln!("output directory: {e}");
        std::process::exit(1)
    }
    let report = Reporter {
        path: output.join("report.json"),
        state: Arc::new(Mutex::new(
            json!({"probe_version": "0.1.0", "wgpu_version": "30.0.1", "requested_backends": format!("{backends:?}"), "require_intel": require_intel, "headless": true, "window_or_surface_created": false, "status": "running", "hardware_validation": "not_tested", "graphics": {"status": "not_tested"}, "compute": {"status": "not_tested"}, "warm_iterations": iterations, "deadline_seconds": deadline_seconds,
        "interpretation": "availability test only; stage timings are CPU wall time including synchronization/readback, not GPU kernel time or representative Catwalk benchmark", "explicit_resource_bytes": {"triangle_texture": SIDE*SIDE*4, "triangle_readback": SIDE*SIDE*4, "compute_three_buffers": COMPUTE_BYTES*3, "driver_overhead": "not included"}}),
        )),
    };
    let watchdog = report.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(deadline_seconds));
        let _ = watchdog.set("status", json!("timeout"));
        let _ = watchdog.set("error", json!("global probe deadline exceeded"));
        eprintln!("probe exceeded {deadline_seconds}s deadline");
        std::process::exit(124);
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&report, &output, iterations, backends, require_intel)
    }));
    let exit_code = match result {
        Ok(Ok(true)) => 0,
        Ok(Ok(false)) => 2,
        Ok(Err(e)) => {
            let _ = report.set("status", json!("failed"));
            let _ = report.set("error", json!(e.to_string()));
            eprintln!("FAIL: {e}");
            1
        }
        Err(e) => {
            let message = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown panic".to_string());
            let _ = report.set("status", json!("failed"));
            let _ = report.set("error", json!(message));
            eprintln!("FAIL: probe panicked");
            1
        }
    };
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expected_transform_handles_positions_and_vectors() {
        assert_eq!(transformed([2.0, 3.0, 4.0, 1.0]), [-3.0, -3.0, 9.0, 1.0]);
        assert_eq!(transformed([2.0, 3.0, 4.0, 0.0]), [-6.0, 1.0, 4.0, 0.0]);
    }
}
