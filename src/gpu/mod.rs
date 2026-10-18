//! Optional headless hardware rendering of shared indexed surface meshes.
//!
//! The prototype supports opaque, untextured triangles, corner colors, authored
//! normals, unlit materials and sidedness. Import, validation and fitting remain
//! on the CPU. Unsupported scenes return an error so callers can use the CPU path.
//! Explicit allocation budgets exclude driver overhead and other live GPU scenes.
//! Blank legacy raster output requests CPU fallback to preserve thin-surface outlines.
//! Distinct placements with identical packed projection rows also request CPU fallback;
//! arbitrary near-coincident surfaces remain subject to finite-precision depth differences.

mod dispatch;
mod prepare;

pub use dispatch::{geometry_thumbnail, thumbnail};

use crate::{Result, instanced::InstancedScene, render::RenderOptions};
use image::RgbaImage;
use prepare::{Draw, Prepared};
use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

/// Maximum duration of one submitted GPU wait and one readback notification wait.
const GPU_WAIT: Duration = Duration::from_secs(5);
/// Default per-scene budget for explicitly allocated GPU buffers and images.
const DEFAULT_ALLOCATION_BYTES: u64 = 256 * 1024 * 1024;
/// Packed vertex ABI, including geometric normals and corner material attributes.
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4];
/// Packed instance ABI, applying projection and normal placement in the vertex shader.
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4];

/// Hardware adapter selection and explicit per-scene allocation budget.
#[derive(Clone, Debug)]
pub struct GpuOptions {
    /// Native graphics backends to enumerate, defaulting to every enabled backend.
    pub backends: wgpu::Backends,
    /// Optional case-insensitive substring of a hardware adapter's name.
    pub adapter_name: Option<String>,
    /// Maximum explicit bytes for one uploaded scene, excluding driver overhead.
    pub max_allocation_bytes: u64,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            backends: wgpu::Backends::all(),
            adapter_name: None,
            max_allocation_bytes: DEFAULT_ALLOCATION_BYTES,
        }
    }
}

/// Scoped device errors, popped in reverse order after all work has been checked.
struct ErrorScopes {
    /// Validation failures captured before they become uncaught panics.
    validation: wgpu::ErrorScopeGuard,
    /// Allocation failures captured independently from validation errors.
    allocation: wgpu::ErrorScopeGuard,
    /// Internal backend failures captured independently from caller validation.
    internal: wgpu::ErrorScopeGuard,
}

impl ErrorScopes {
    /// Capture every error category for operations on the current thread.
    fn new(device: &wgpu::Device) -> Self {
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        Self {
            validation,
            allocation,
            internal,
        }
    }

    /// Pop every scope and return any captured backend error.
    fn finish(self) -> Result<()> {
        let errors = [
            pollster::block_on(self.validation.pop()),
            pollster::block_on(self.allocation.pop()),
            pollster::block_on(self.internal.pop()),
        ];
        let messages: Vec<_> = errors
            .into_iter()
            .flatten()
            .map(|e| e.to_string())
            .collect();
        if messages.is_empty() {
            Ok(())
        } else {
            Err(format!("GPU operation failed: {}", messages.join("; ")).into())
        }
    }
}

/// A mesh's device-resident vertex/index storage, shared by every occurrence.
struct MeshBuffers {
    /// Packed complete vertex attributes, split only at authored seams.
    vertices: wgpu::Buffer,
    /// Original ordered triangle corners into the packed vertices.
    indices: wgpu::Buffer,
    /// Number of packed indices representable by one indexed draw.
    count: u32,
}

/// CPU wall time spent preparing a scene and creating its reusable GPU resources.
///
/// Resource creation includes scoped driver validation but does not add a DMA
/// completion fence. Pending uploads may finish during the first render call.
#[derive(Clone, Copy, Debug)]
pub struct GpuUploadTimings {
    /// CPU validation, fitting and packing performed by the scene preparation step.
    pub cpu_preparation: Duration,
    /// CPU wall time creating GPU resources and checking scoped backend errors.
    pub resource_creation: Duration,
}

/// Uploaded reusable geometry and render targets belonging to one renderer.
///
/// No expanded world-space triangle array or prepared CPU vertex arrays are kept.
/// Retain this object to render the same scene repeatedly without another upload.
pub struct GpuScene {
    /// Renderer identity preventing accidental cross-device resource submission.
    owner: Arc<()>,
    /// Unique mesh buffers; degenerate-only meshes have no buffer allocation.
    meshes: Vec<Option<MeshBuffers>>,
    /// Ordered shared mesh references and corresponding instance attributes.
    draws: Vec<Draw>,
    /// Packed per-occurrence placement and normal transforms.
    instances: wgpu::Buffer,
    /// Uniform binding retained through repeated geometry draws.
    geometry_bind: wgpu::BindGroup,
    /// Uniform and supersampled-image binding used by the final resolve pass.
    resolve_bind: wgpu::BindGroup,
    /// Supersampled straight-alpha encoded render target.
    color: wgpu::TextureView,
    /// Supersampled floating-point depth target.
    depth: wgpu::TextureView,
    /// Final resolved RGBA8 image, copied back after rendering.
    resolved: wgpu::Texture,
    /// View into the final image used by the resolve render pass.
    resolved_view: wgpu::TextureView,
    /// Reusable padded readback allocation for the finished thumbnail only.
    readback: wgpu::Buffer,
    /// Encoded straight-alpha clear color.
    background: wgpu::Color,
    /// Width of the final thumbnail.
    width: u32,
    /// Height of the final thumbnail.
    height: u32,
    /// Bytes per padded readback row, aligned to the WebGPU copy requirement.
    padded_row: u32,
    /// Total explicit scene buffer and texture bytes, excluding drivers.
    allocation_bytes: u64,
    /// Upload stage measurements without an extra GPU synchronization fence.
    upload_timings: GpuUploadTimings,
    /// Whether visible legacy outlines may be required when no sample is covered.
    has_legacy_surface: bool,
    /// Exact encoded RGBA background used to identify empty legacy raster output.
    background_bytes: [u8; 4],
}

impl GpuScene {
    /// Return explicit scene buffer and texture bytes, excluding driver overhead.
    #[must_use]
    pub fn explicit_allocation_bytes(&self) -> u64 {
        self.allocation_bytes
    }

    /// Return CPU preparation and GPU resource-creation wall times for this upload.
    ///
    /// Pending transfers may complete during the first draw; these are not GPU
    /// execution timings or a promise that uploads finished before rendering.
    #[must_use]
    pub fn upload_timings(&self) -> GpuUploadTimings {
        self.upload_timings
    }
}

/// Supported adapter capabilities and the limits/features enabled on this device.
///
/// Adapter support can exceed the deliberately restricted device request. These
/// values describe the renderer's actual wgpu version and selected graphics API.
#[derive(Clone, Debug)]
pub struct GpuCapabilities {
    /// Maximum capabilities reported by the selected physical adapter.
    pub adapter_limits: wgpu::Limits,
    /// Limits enabled by this renderer's successful device request.
    pub device_limits: wgpu::Limits,
    /// Optional features supported by the selected physical adapter.
    pub adapter_features: wgpu::Features,
    /// Optional features enabled on this renderer's device.
    pub device_features: wgpu::Features,
    /// Downlevel flags and shader model reported by the selected adapter.
    pub downlevel: wgpu::DownlevelCapabilities,
}

/// Reusable headless hardware device and pipelines, independent of any window.
///
/// CPU/software and unknown adapter types are rejected. Callers decide whether an
/// unavailable device or unsupported scene should trigger their CPU fallback.
pub struct GpuRenderer {
    /// Instance retained for native backend lifetime and headless context ownership.
    _instance: wgpu::Instance,
    /// Hardware and exact API identity selected during initialization.
    info: wgpu::AdapterInfo,
    /// Adapter support and actual enabled device limits/features.
    capabilities: GpuCapabilities,
    /// Device reused for every uploaded scene and thumbnail.
    device: wgpu::Device,
    /// Submission queue reused alongside the device.
    queue: wgpu::Queue,
    /// Shader pipeline applying placement, normals and opaque surface shading.
    geometry_pipeline: wgpu::RenderPipeline,
    /// Shader pipeline resolving supersamples without geometry readback.
    resolve_pipeline: wgpu::RenderPipeline,
    /// Explicit layout for the camera uniform used while drawing surfaces.
    geometry_layout: wgpu::BindGroupLayout,
    /// Explicit layout for camera settings and supersampled color input.
    resolve_layout: wgpu::BindGroupLayout,
    /// Validated per-scene allocation ceiling.
    budget: u64,
    /// Identity copied into uploaded scenes to validate device ownership.
    owner: Arc<()>,
    /// Serialize map/reuse operations and device error scopes across callers.
    operation: Mutex<()>,
}

/// Validate that an enumerated adapter identifies physical GPU hardware.
fn hardware(info: &wgpu::AdapterInfo) -> bool {
    let identity =
        format!("{} {} {}", info.name, info.driver, info.driver_info).to_ascii_lowercase();
    matches!(
        info.device_type,
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::DiscreteGpu
    ) && ![
        "llvmpipe",
        "softpipe",
        "swrast",
        "swiftshader",
        "software",
        "lavapipe",
    ]
    .iter()
    .any(|name| identity.contains(name))
}

/// One immutable uniform binding visible to vertex and fragment stages.
fn uniform_layout() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(32),
        },
        count: None,
    }
}

/// Construct one texture with no mipmaps, multisampling or presentation surface.
fn texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl GpuRenderer {
    /// Select a physical hardware adapter and create reusable headless pipelines.
    ///
    /// # Errors
    /// Returns an error if no matching hardware exists, requested backends are
    /// empty, required device limits are absent, or device/pipeline creation fails.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep native pipeline descriptors and their scoped validation together"
    )]
    pub fn new(options: GpuOptions) -> Result<Self> {
        if options.backends.is_empty() || options.max_allocation_bytes == 0 {
            return Err("GPU backends and allocation budget must be nonempty".into());
        }
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = options.backends;
        let instance = wgpu::Instance::new(descriptor);
        let adapters = pollster::block_on(instance.enumerate_adapters(options.backends));
        let requested_name = options.adapter_name.map(|name| name.to_ascii_lowercase());
        let available = adapters
            .iter()
            .map(|a| {
                let info = a.get_info();
                format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let adapter = adapters
            .into_iter()
            .find(|a| {
                let info = a.get_info();
                hardware(&info)
                    && requested_name
                        .as_ref()
                        .is_none_or(|name| info.name.to_ascii_lowercase().contains(name))
            })
            .ok_or_else(|| format!("no matching physical GPU adapter; enumerated: {available}"))?;
        Self::new_from_adapter(instance, &adapter, options.max_allocation_bytes)
    }

    /// Build checked pipelines after public hardware selection, or for test-only software validation.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep native pipeline descriptors and their scoped validation together"
    )]
    fn new_from_adapter(
        instance: wgpu::Instance,
        adapter: &wgpu::Adapter,
        budget: u64,
    ) -> Result<Self> {
        let info = adapter.get_info();
        let supported = adapter.limits();
        let adapter_features = adapter.features();
        let downlevel = adapter.get_downlevel_capabilities();
        if supported.max_vertex_attributes < 12 || supported.max_vertex_buffers < 2 {
            return Err(
                "GPU adapter cannot supply twelve vertex attributes and two vertex buffers".into(),
            );
        }
        for (format, usage) in [
            (
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            ),
            (
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
        ] {
            if !adapter
                .get_texture_format_features(format)
                .allowed_usages
                .contains(usage)
            {
                return Err(format!("GPU adapter lacks required {format:?} image usages").into());
            }
        }
        let mut required = wgpu::Limits::downlevel_webgl2_defaults()
            .using_resolution(supported.clone())
            .using_alignment(supported.clone());
        required.max_vertex_attributes = 12;
        required.max_vertex_buffers = 2;
        required.max_inter_stage_shader_variables = 4;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("Catwalk headless hardware renderer"),
                required_features: wgpu::Features::empty(),
                required_limits: required,
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }))?;
        let capabilities = GpuCapabilities {
            adapter_limits: supported,
            device_limits: device.limits(),
            adapter_features,
            device_features: device.features(),
            downlevel,
        };
        let scopes = ErrorScopes::new(&device);
        let geometry_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Catwalk geometry camera"),
            entries: &[uniform_layout()],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Catwalk final resolve"),
            entries: &[
                uniform_layout(),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let geometry_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Catwalk geometry pipeline layout"),
                bind_group_layouts: &[Some(&geometry_layout)],
                immediate_size: 0,
            });
        let resolve_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Catwalk resolve pipeline layout"),
                bind_group_layouts: &[Some(&resolve_layout)],
                immediate_size: 0,
            });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Catwalk placements, shading and resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let target = [Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let geometry_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Catwalk shared indexed surfaces"),
            layout: Some(&geometry_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("mesh_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 68,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: 112,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRIBUTES,
                    },
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("mesh_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });
        let resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Catwalk premultiplied final resolve"),
            layout: Some(&resolve_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("resolve_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("resolve_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });
        scopes.finish()?;
        Ok(Self {
            _instance: instance,
            info,
            capabilities,
            device,
            queue,
            geometry_pipeline,
            resolve_pipeline,
            geometry_layout,
            resolve_layout,
            budget,
            owner: Arc::new(()),
            operation: Mutex::new(()),
        })
    }

    /// Return hardware identity including the exact graphics backend in use.
    #[must_use]
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.info.clone()
    }

    /// Return supported adapter capabilities and actual enabled device limits/features.
    #[must_use]
    pub fn capabilities(&self) -> GpuCapabilities {
        self.capabilities.clone()
    }

    /// Validate, fit and upload shared geometry with reusable output allocations.
    ///
    /// The temporary prepared CPU vertices are released before this method returns.
    ///
    /// # Errors
    /// Returns an error for unsupported materials/primitives, invalid or imprecise
    /// geometry, exceeded explicit allocation/device limits, or backend failures.
    pub fn upload(&self, scene: &InstancedScene, options: &RenderOptions) -> Result<GpuScene> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "GPU renderer operation lock is poisoned")?;
        let preparation_started = Instant::now();
        let prepared = Prepared::new(scene, options, self.budget)?;
        let cpu_preparation = preparation_started.elapsed();
        self.check_limits(&prepared)?;
        let has_legacy_surface = scene.instances.iter().any(|instance| {
            scene.meshes.get(instance.mesh).is_some_and(|mesh| {
                mesh.faces
                    .iter()
                    .enumerate()
                    .any(|(index, _)| !mesh.appearances.contains_key(&index))
            })
        });
        let background_bytes = if options.transparent {
            [0; 4]
        } else {
            [
                options.background[0],
                options.background[1],
                options.background[2],
                255,
            ]
        };
        let resource_started = Instant::now();
        let scopes = ErrorScopes::new(&self.device);
        let result = self.upload_prepared(prepared, has_legacy_surface, background_bytes);
        scopes.finish()?;
        let mut uploaded = result?;
        uploaded.upload_timings = GpuUploadTimings {
            cpu_preparation,
            resource_creation: resource_started.elapsed(),
        };
        Ok(uploaded)
    }

    /// Check individual device allocations before creating any scene resources.
    fn check_limits(&self, prepared: &Prepared) -> Result<()> {
        let limits = self.device.limits();
        if prepared.width > limits.max_texture_dimension_2d
            || prepared.height > limits.max_texture_dimension_2d
        {
            return Err("GPU sample image dimensions exceed adapter limits".into());
        }
        let max_buffer = limits.max_buffer_size;
        let instance_bytes = u64::try_from(prepared.instances.len())?
            .checked_mul(112)
            .ok_or("GPU instance buffer size overflow")?;
        let readback_bytes = u64::from(prepared.uniforms.settings[1])
            .checked_mul(4)
            .ok_or("GPU readback size overflow")?
            .div_ceil(256)
            * 256
            * u64::from(prepared.uniforms.settings[2]);
        if instance_bytes > max_buffer || readback_bytes > max_buffer {
            return Err("GPU instance or readback buffer exceeds adapter limits".into());
        }
        for mesh in &prepared.meshes {
            if u64::try_from(mesh.vertices.len())? * 68 > max_buffer
                || u64::try_from(mesh.indices.len())? * 4 > max_buffer
            {
                return Err("GPU mesh buffer exceeds adapter limits".into());
            }
        }
        Ok(())
    }

    /// Upload already bounded CPU data without retaining expanded geometry.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep allocation creation and the returned resource ownership together"
    )]
    fn upload_prepared(
        &self,
        prepared: Prepared,
        has_legacy_surface: bool,
        background_bytes: [u8; 4],
    ) -> Result<GpuScene> {
        let mut meshes = Vec::new();
        meshes.try_reserve_exact(prepared.meshes.len())?;
        for mesh in &prepared.meshes {
            meshes.push(if mesh.indices.is_empty() {
                None
            } else {
                Some(MeshBuffers {
                    vertices: self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Catwalk shared vertices"),
                            contents: bytemuck::cast_slice(&mesh.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                    indices: self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Catwalk shared indices"),
                            contents: bytemuck::cast_slice(&mesh.indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                    count: u32::try_from(mesh.indices.len())?,
                })
            });
        }
        let instances = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Catwalk instance transforms"),
                contents: bytemuck::cast_slice(&prepared.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let uniforms = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Catwalk view and resolve uniforms"),
                contents: bytemuck::bytes_of(&prepared.uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let color = texture(
            &self.device,
            "Catwalk supersampled color",
            prepared.width,
            prepared.height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = texture(
            &self.device,
            "Catwalk supersampled depth",
            prepared.width,
            prepared.height,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        let width = prepared.uniforms.settings[1];
        let height = prepared.uniforms.settings[2];
        let resolved = texture(
            &self.device,
            "Catwalk resolved thumbnail",
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = (width * 4).div_ceil(256) * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Catwalk final thumbnail readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let geometry_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Catwalk geometry bindings"),
            layout: &self.geometry_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let resolve_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Catwalk resolve bindings"),
            layout: &self.resolve_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
            ],
        });
        let [r, g, b, a] = prepared.background;
        Ok(GpuScene {
            owner: Arc::clone(&self.owner),
            meshes,
            draws: prepared.draws,
            instances,
            geometry_bind,
            resolve_bind,
            color,
            depth,
            resolved,
            resolved_view,
            readback,
            background: wgpu::Color { r, g, b, a },
            width,
            height,
            padded_row,
            allocation_bytes: prepared.allocation_bytes,
            upload_timings: GpuUploadTimings {
                cpu_preparation: Duration::ZERO,
                resource_creation: Duration::ZERO,
            },
            has_legacy_surface,
            background_bytes,
        })
    }

    /// Render an uploaded scene, reading back only the final resolved thumbnail.
    ///
    /// # Errors
    /// Returns an error for resources from another renderer, poisoned operation
    /// state, backend failures, allocation failure, a GPU/readback timeout, or
    /// blank legacy output requiring the CPU renderer's subpixel outline fallback.
    pub fn render(&self, scene: &GpuScene) -> Result<RgbaImage> {
        if !Arc::ptr_eq(&self.owner, &scene.owner) {
            return Err("GPU scene belongs to a different renderer".into());
        }
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "GPU renderer operation lock is poisoned")?;
        let scopes = ErrorScopes::new(&self.device);
        let result = self.render_inner(scene);
        scopes.finish()?;
        result
    }

    /// Encode ordered geometry, GPU resolve, final copy and bounded map/readback.
    fn render_inner(&self, scene: &GpuScene) -> Result<RgbaImage> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Catwalk thumbnail commands"),
            });
        {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: &scene.color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(scene.background),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Catwalk ordered indexed instances"),
                color_attachments: &colors,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &scene.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.geometry_pipeline);
            pass.set_bind_group(0, &scene.geometry_bind, &[]);
            for draw in &scene.draws {
                let mesh = scene
                    .meshes
                    .get(draw.mesh)
                    .and_then(Option::as_ref)
                    .ok_or("GPU draw references a missing mesh")?;
                let offset = u64::from(draw.instance) * 112;
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_vertex_buffer(1, scene.instances.slice(offset..offset + 112));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, 0..1);
            }
        }
        {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: &scene.resolved_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Catwalk GPU supersample resolve"),
                color_attachments: &colors,
                ..Default::default()
            });
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &scene.resolve_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &scene.resolved,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &scene.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(scene.padded_row),
                    rows_per_image: Some(scene.height),
                },
            },
            wgpu::Extent3d {
                width: scene.width,
                height: scene.height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let result = self.read_image(scene, submission);
        scene.readback.unmap();
        result
    }

    /// Wait for GPU completion, copy unpadded output rows and release the map view.
    fn read_image(&self, scene: &GpuScene, submission: wgpu::SubmissionIndex) -> Result<RgbaImage> {
        let (sender, receiver) = mpsc::sync_channel(1);
        scene
            .readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = sender.send(result);
            });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(GPU_WAIT),
        })?;
        receiver.recv_timeout(GPU_WAIT)??;
        let view = scene.readback.get_mapped_range(..);
        let row_bytes = usize::try_from(scene.width)? * 4;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(row_bytes * usize::try_from(scene.height)?)?;
        for row in view.chunks_exact(usize::try_from(scene.padded_row)?) {
            rgba.extend_from_slice(&row[..row_bytes]);
        }
        drop(view);
        let image = RgbaImage::from_raw(scene.width, scene.height, rgba)
            .ok_or("invalid GPU thumbnail output dimensions")?;
        if scene.has_legacy_surface
            && image.pixels().all(|pixel| {
                pixel
                    .0
                    .iter()
                    .zip(scene.background_bytes)
                    .all(|(value, background)| value.abs_diff(background) <= 1)
            })
        {
            return Err("GPU thumbnail has no visible legacy surface samples; use CPU fallback to preserve subpixel outlines".into());
        }
        Ok(image)
    }
}

#[cfg(test)]
/// Shader validation checks may use a software device without claiming acceleration.
mod tests {
    use super::{DEFAULT_ALLOCATION_BYTES, ErrorScopes, GpuOptions, GpuRenderer};
    use crate::{
        instanced::{InstancedScene, MeshInstance},
        render::{self, RenderOptions},
        scene::{Scene, Triangle, TriangleAppearance, Vec3},
    };
    use image::RgbaImage;

    /// Invalid configuration fails before native driver initialization.
    #[test]
    fn rejects_empty_backend_selection() {
        let result = GpuRenderer::new(GpuOptions {
            backends: wgpu::Backends::empty(),
            ..GpuOptions::default()
        });
        assert!(result.is_err());
    }

    /// Parse and validate every WGSL entry point using any available GL device.
    #[test]
    fn validates_shader_on_available_gl_device() -> crate::Result<()> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::GL;
        let instance = wgpu::Instance::new(descriptor);
        let Some(adapter) = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::GL))
            .into_iter()
            .next()
        else {
            if std::env::var_os("CATWALK_REQUIRE_GPU_TEST_ADAPTER").is_some() {
                return Err("GPU shader validation requires an available GL test adapter".into());
            }
            eprintln!(
                "no GL adapter available for shader validation; hardware checks run separately"
            );
            return Ok(());
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("shader validation only, including software devices"),
                required_limits:
                    wgpu::Limits::downlevel_webgl2_defaults().using_alignment(adapter.limits()),
                ..wgpu::DeviceDescriptor::default()
            }))?;
        let scopes = ErrorScopes::new(&device);
        let _shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("validate every Catwalk WGSL entry point"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        scopes.finish()
    }
    /// Select the backend used by software-default or explicitly hardware-required tests.
    fn correctness_backend() -> crate::Result<wgpu::Backends> {
        let backend = match std::env::var("CATWALK_GPU_TEST_BACKEND") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => "gl".to_string(),
            Err(error) => return Err(error.into()),
        };
        match backend.to_ascii_lowercase().as_str() {
            "gl" | "gles" => Ok(wgpu::Backends::GL),
            "dx12" => Ok(wgpu::Backends::DX12),
            "vulkan" => Ok(wgpu::Backends::VULKAN),
            "metal" => Ok(wgpu::Backends::METAL),
            _ => Err(format!("unsupported CATWALK_GPU_TEST_BACKEND: {backend}").into()),
        }
    }

    /// Build a correctness device; defaults to software-first GL, with opt-in hardware validation.
    fn correctness_renderer() -> crate::Result<Option<GpuRenderer>> {
        let backends = correctness_backend()?;
        let require_hardware = std::env::var_os("CATWALK_REQUIRE_HARDWARE_GPU_TEST").is_some();
        let adapter_name = std::env::var("CATWALK_GPU_TEST_ADAPTER")
            .ok()
            .map(|name| name.to_ascii_lowercase());
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
        adapters.retain(|adapter| {
            let info = adapter.get_info();
            (!require_hardware || super::hardware(&info))
                && adapter_name
                    .as_ref()
                    .is_none_or(|name| info.name.to_ascii_lowercase().contains(name))
        });
        adapters.sort_by_key(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu);
        let Some(adapter) = adapters.into_iter().next() else {
            if require_hardware || std::env::var_os("CATWALK_REQUIRE_GPU_TEST_ADAPTER").is_some() {
                return Err(format!("GPU image correctness requires an eligible {backends:?} test adapter (hardware_required={require_hardware})").into());
            }
            eprintln!(
                "SKIP image correctness: no eligible {backends:?} adapter (hardware_required={require_hardware})"
            );
            return Ok(None);
        };
        let info = adapter.get_info();
        eprintln!(
            "correctness adapter: {} / {:?} / {:?}; hardware_required={require_hardware}; no performance claim",
            info.name, info.backend, info.device_type
        );
        if info.device_type == wgpu::DeviceType::Cpu {
            assert!(
                GpuRenderer::new(GpuOptions {
                    backends,
                    adapter_name: Some(info.name),
                    ..GpuOptions::default()
                })
                .is_err(),
                "public renderer must reject the software adapter used by correctness tests"
            );
        }
        GpuRenderer::new_from_adapter(instance, &adapter, DEFAULT_ALLOCATION_BYTES).map(Some)
    }

    /// Build one surface with optional material, independent of an imported format.
    fn surface(appearance: Option<TriangleAppearance>) -> crate::Result<InstancedScene> {
        let mut scene = Scene::default();
        scene.triangles.push(Triangle {
            vertices: [
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            color: Some([100, 160, 220]),
        });
        if let Some(appearance) = appearance {
            scene.appearances.insert(0, appearance);
        }
        InstancedScene::from_scene(scene)
    }

    /// Compare shading interiors while allowing local raster-edge and byte-rounding differences.
    fn compare_images(label: &str, cpu: &RgbaImage, gpu: &RgbaImage) -> crate::Result<()> {
        assert_eq!(cpu.dimensions(), gpu.dimensions());
        let close = |a: &image::Rgba<u8>, b: &image::Rgba<u8>| {
            a.0.iter().zip(b.0).all(|(a, b)| a.abs_diff(b) <= 4)
        };
        let nearby = |image: &RgbaImage, x: u32, y: u32, wanted: &image::Rgba<u8>| {
            (y.saturating_sub(1)..=y.saturating_add(1).min(image.height() - 1)).any(|ny| {
                (x.saturating_sub(1)..=x.saturating_add(1).min(image.width() - 1))
                    .any(|nx| close(wanted, image.get_pixel(nx, ny)))
            })
        };
        let mut error_sum = 0_u32;
        let mut unmatched = 0_u32;
        let mut intersection = 0_u32;
        let mut union = 0_u32;
        for (x, y, expected) in cpu.enumerate_pixels() {
            let actual = gpu.get_pixel(x, y);
            error_sum += expected
                .0
                .iter()
                .zip(actual.0)
                .map(|(a, b)| u32::from(a.abs_diff(b)))
                .sum::<u32>();
            if !close(expected, actual)
                && (!nearby(gpu, x, y, expected) || !nearby(cpu, x, y, actual))
            {
                unmatched += 1;
            }
            intersection += u32::from(expected[3] > 0 && actual[3] > 0);
            union += u32::from(expected[3] > 0 || actual[3] > 0);
        }
        let mean_error = f64::from(error_sum) / f64::from(u32::try_from(cpu.as_raw().len())?);
        let pixels = cpu.width() * cpu.height();
        eprintln!(
            "{label}: mean byte error={mean_error:.4}, unmatched outside one-pixel edge tolerance={unmatched}/{pixels}, coverage intersection/union={intersection}/{union}"
        );
        if mean_error > 2.0
            || unmatched > pixels / 200
            || (union != 0 && u64::from(intersection) * 1000 < u64::from(union) * 970)
        {
            let output = std::env::temp_dir().join("catwalk-gpu-correctness-failures");
            let _ = std::fs::create_dir_all(&output);
            let _ = cpu.save(output.join(format!("{label}-cpu.png")));
            let _ = gpu.save(output.join(format!("{label}-gpu.png")));
            return Err(format!("{label}: CPU/GPU image mismatch (mean byte error {mean_error}, unmatched {unmatched}); diagnostic PNGs in {}", output.display()).into());
        }
        Ok(())
    }

    /// Exercise real rendering on default software GL or an explicitly selected hardware backend.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the named correctness matrix and its independent CPU comparison together"
    )]
    fn software_gl_matches_cpu_supported_surfaces() -> crate::Result<()> {
        let Some(renderer) = correctness_renderer()? else {
            return Ok(());
        };
        let mut cases = Vec::new();
        let base_options = RenderOptions {
            width: 128,
            height: 96,
            ..RenderOptions::default()
        };
        for transparent in [false, true] {
            for supersampling in [1, 2, 4] {
                cases.push((
                    format!("legacy-ss{supersampling}-transparent-{transparent}"),
                    surface(None)?,
                    RenderOptions {
                        transparent,
                        supersampling,
                        ..base_options.clone()
                    },
                ));
            }
        }
        let mut shared = surface(None)?;
        let mut red_mesh = shared.meshes[0].clone();
        red_mesh.faces[0].color = Some([220, 80, 80]);
        shared.meshes.push(red_mesh);
        let mut placed = MeshInstance::default();
        placed.transform[0][3] = 2.5;
        shared.instances = vec![
            MeshInstance {
                mesh: 1,
                ..MeshInstance::default()
            },
            MeshInstance::default(),
            placed,
        ];
        cases.push((
            "ordered-shared-instances-depth-ties".to_string(),
            shared,
            base_options.clone(),
        ));
        let material = TriangleAppearance {
            base_color: [0.65, 0.35, 0.2, 1.0],
            ..TriangleAppearance::default()
        };
        cases.push((
            "opaque-material".to_string(),
            surface(Some(material))?,
            base_options.clone(),
        ));
        let colored = TriangleAppearance {
            base_color: [0.6, 0.8, 0.4, 1.0],
            colors: Some([
                [1.0, 0.25, 0.1, 1.0],
                [0.25, 1.0, 0.1, 1.0],
                [0.25, 0.25, 1.0, 1.0],
            ]),
            normals: Some([
                Vec3::new(-0.3, 0.0, 1.0),
                Vec3::new(0.3, 0.0, 1.0),
                Vec3::new(0.0, 0.4, 1.0),
            ]),
            ..TriangleAppearance::default()
        };
        cases.push((
            "corner-colors-authored-normals".to_string(),
            surface(Some(colored.clone()))?,
            base_options.clone(),
        ));
        let mut unlit = colored.clone();
        unlit.unlit = true;
        cases.push((
            "unlit-linear-resolve-transparent".to_string(),
            surface(Some(unlit))?,
            RenderOptions {
                supersampling: 3,
                transparent: true,
                ..base_options.clone()
            },
        ));
        let mut single = colored.clone();
        single.double_sided = false;
        cases.push((
            "single-sided-front".to_string(),
            surface(Some(single.clone()))?,
            base_options.clone(),
        ));
        let mut back = surface(Some(single))?;
        back.instances[0].reverse_winding = true;
        cases.push((
            "single-sided-culled".to_string(),
            back,
            RenderOptions {
                transparent: true,
                ..base_options.clone()
            },
        ));
        for reverse in [false, true] {
            let mut mirrored = surface(Some(colored.clone()))?;
            mirrored.instances[0].transform = [
                [-1.3, 0.4, 0.0, 0.25],
                [0.0, 0.7, 0.2, -0.2],
                [0.0, 0.1, 1.1, 0.5],
                [0.0, 0.0, 0.0, 1.0],
            ];
            mirrored.instances[0].reverse_winding = reverse;
            cases.push((
                format!("mirrored-nonuniform-shear-reverse-{reverse}"),
                mirrored,
                RenderOptions {
                    transparent: true,
                    ..base_options.clone()
                },
            ));
        }
        // Embed the owned fixture so this same test binary can run on the NAS
        // or Windows without the build machine's absolute source checkout path.
        let fixture_directory = std::env::temp_dir().join(format!(
            "catwalk-gpu-correctness-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&fixture_directory)?;
        let fixture = fixture_directory.join("gear.obj");
        std::fs::write(&fixture, include_bytes!("../../examples/models/gear.obj"))?;
        let gear = crate::formats::load(&fixture);
        let _ = std::fs::remove_file(&fixture);
        let _ = std::fs::remove_dir(&fixture_directory);
        let gear = gear?;
        cases.push((
            "imported-gear".to_string(),
            InstancedScene::from_scene(gear)?,
            RenderOptions {
                width: 192,
                height: 160,
                supersampling: 2,
                ..RenderOptions::default()
            },
        ));
        for (label, scene, options) in cases {
            let cpu = render::render(&scene.to_scene()?, &options)?;
            let uploaded = renderer.upload(&scene, &options)?;
            let gpu = renderer.render(&uploaded)?;
            compare_images(&label, &cpu, &gpu)?;
            assert_eq!(
                gpu.as_raw(),
                renderer.render(&uploaded)?.as_raw(),
                "reused-device draw changed {label}"
            );
        }
        Ok(())
    }

    /// Subpixel legacy fallback is checked on software GL or explicitly selected hardware.
    #[test]
    fn software_gl_preserves_thin_legacy_fallback() -> crate::Result<()> {
        let Some(renderer) = correctness_renderer()? else {
            return Ok(());
        };
        let mut scene = Scene::default();
        scene.triangles.push(Triangle {
            vertices: [
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1e-11, 0.0),
            ],
            color: Some([100, 160, 220]),
        });
        let options = RenderOptions {
            width: 128,
            height: 96,
            supersampling: 1,
            transparent: true,
            ..RenderOptions::default()
        };
        let cpu = render::render(&scene, &options)?;
        assert!(
            cpu.pixels().any(|pixel| pixel[3] != 0),
            "CPU thin-surface outline must be visible"
        );
        let shared = InstancedScene::from_scene(scene)?;
        let uploaded = renderer.upload(&shared, &options)?;
        let error = renderer
            .render(&uploaded)
            .expect_err("collapsed legacy surface must request CPU fallback");
        assert!(error.to_string().contains("CPU fallback"));
        Ok(())
    }
    /// Build independently colored parallel panels separated along the active camera direction.
    fn near_coplanar_panels(separation: f64) -> crate::Result<InstancedScene> {
        let positions = [
            Vec3::new(0.0, -2.0, -2.0),
            Vec3::new(2.0, 0.0, -2.0),
            Vec3::new(0.0, 2.0, 2.0),
            Vec3::new(-2.0, 0.0, 2.0),
        ];
        let mut source = Scene::default();
        for corners in [[0, 1, 2], [0, 2, 3]] {
            source.triangles.push(Triangle {
                vertices: corners.map(|index| positions[index]),
                color: None,
            });
            source.appearances.insert(
                source.triangles.len() - 1,
                TriangleAppearance {
                    base_color: [0.8, 0.1, 0.1, 1.0],
                    unlit: true,
                    ..TriangleAppearance::default()
                },
            );
        }
        let mut scene = InstancedScene::from_scene(source)?;
        let mut front = scene.meshes[0].clone();
        for appearance in front.appearances.values_mut() {
            appearance.base_color = [0.1, 0.1, 0.8, 1.0];
        }
        scene.meshes.push(front);
        let mut placed = MeshInstance {
            mesh: 1,
            ..MeshInstance::default()
        };
        placed.transform[0][3] = separation;
        placed.transform[1][3] = -separation;
        placed.transform[2][3] = separation;
        scene.instances.push(placed);
        Ok(scene)
    }

    /// A demonstrated collapsed placement requests CPU fallback while source ties remain valid.
    #[test]
    fn software_gl_near_coplanar_visibility_requires_precision_fallback() -> crate::Result<()> {
        let Some(renderer) = correctness_renderer()? else {
            return Ok(());
        };
        let options = RenderOptions {
            width: 128,
            height: 96,
            supersampling: 1,
            ..RenderOptions::default()
        };
        let scene = near_coplanar_panels(1e-9)?;
        let cpu = render::render(&scene.to_scene()?, &options)?;
        let center = cpu.get_pixel(options.width / 2, options.height / 2);
        assert!(
            center[2] > center[0],
            "CPU must resolve the closer blue panel in front"
        );
        let error = match renderer.upload(&scene, &options) {
            Ok(_) => return Err("collapsed placements must request CPU fallback".into()),
            Err(error) => error,
        };
        eprintln!("near-coplanar GPU admission rejected: {error}");
        assert!(error.to_string().contains("same GPU projection"));
        assert!(error.to_string().contains("CPU fallback"));

        let mut different_basis = near_coplanar_panels(0.0)?;
        for position in &mut different_basis.meshes[1].positions {
            position.x += 8.0;
        }
        different_basis.instances[1].transform[0][3] = -8.0;
        for (label, scene) in [
            ("exact-source-placement-ties", near_coplanar_panels(0.0)?),
            ("distinct-mesh-local-bases", different_basis),
            ("resolvable-depth-separation", near_coplanar_panels(1e-3)?),
        ] {
            let cpu = render::render(&scene.to_scene()?, &options)?;
            let uploaded = renderer.upload(&scene, &options)?;
            compare_images(label, &cpu, &renderer.render(&uploaded)?)?;
        }
        Ok(())
    }
}
