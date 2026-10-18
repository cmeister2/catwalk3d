//! Optional GPU geometry dispatch retaining saved-image preference and complete CPU fallback.

use super::{GpuOptions, GpuRenderer};
use crate::{
    Result, formats,
    instanced::InstancedScene,
    render::{self, RenderOptions},
    scene::Scene,
};
use image::RgbaImage;
use std::path::Path;

/// Convert a backend panic into a failure before selecting complete-scene fallback.
fn attempt<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown backend panic");
        format!("GPU backend failed: {message}")
    })?
}

/// Render fresh model geometry with an optional complete-scene CPU fallback.
///
/// Initialization failure falls back before importing shared geometry. Unsupported
/// GPU scene features or execution failure expand the complete checked scene for
/// CPU rendering. The GPU prototype supports opaque untextured surfaces; points,
/// lines, alpha coverage and textures currently require CPU fallback. XY drawings
/// retain their existing camera through the CPU path.
///
/// # Errors
/// Returns an error for malformed geometry or exceeded import/CPU limits, or a
/// hardware/feature/render failure when `allow_cpu_fallback` is false. Fallback
/// can still fail if the scene exceeds the CPU renderer's resource limits.
pub fn geometry_thumbnail(
    path: &Path,
    options: &RenderOptions,
    gpu: GpuOptions,
    allow_cpu_fallback: bool,
) -> Result<RgbaImage> {
    geometry_from_source(path, options, gpu, allow_cpu_fallback, None)
}

/// Render an already decoded volume on CPU, or import fresh model geometry with its existing camera.
fn cpu_geometry(path: &Path, options: &RenderOptions, decoded: Option<Scene>) -> Result<RgbaImage> {
    if let Some(scene) = decoded {
        render::render(&scene, options)
    } else {
        crate::geometry_thumbnail(path, options)
    }
}

/// Select hardware for a model path or a scalar TIFF scene already decoded by preview dispatch.
fn geometry_from_source(
    path: &Path,
    options: &RenderOptions,
    gpu: GpuOptions,
    allow_cpu_fallback: bool,
    decoded: Option<Scene>,
) -> Result<RgbaImage> {
    if matches!(formats::extension(path)?.as_str(), "lbrn" | "lbrn2") {
        if allow_cpu_fallback {
            return cpu_geometry(path, options, decoded);
        }
        return Err(
            "GPU prototype does not support the XY drawing camera; select auto or CPU".into(),
        );
    }
    let renderer = match attempt(|| GpuRenderer::new(gpu)) {
        Ok(renderer) => renderer,
        Err(_) if allow_cpu_fallback => return cpu_geometry(path, options, decoded),
        Err(error) => return Err(error),
    };
    let source = if let Some(scene) = decoded {
        InstancedScene::from_scene(scene)
    } else {
        formats::load_instanced(path)
    };
    let scene = match source {
        Ok(scene) => scene,
        Err(_) if allow_cpu_fallback => {
            drop(renderer);
            return crate::geometry_thumbnail(path, options);
        }
        Err(error) => return Err(error),
    };
    let gpu_scene = match attempt(|| renderer.upload(&scene, options)) {
        Ok(scene) => scene,
        Err(_) if allow_cpu_fallback => {
            drop(renderer);
            let expanded = scene.to_scene()?;
            drop(scene);
            return render::render(&expanded, options);
        }
        Err(error) => return Err(error),
    };
    if !allow_cpu_fallback {
        // Required mode cannot use the CPU representation after upload. Match
        // the profiling API's ownership before allocating the final image.
        drop(scene);
        return attempt(|| renderer.render(&gpu_scene));
    }
    match attempt(|| renderer.render(&gpu_scene)) {
        Ok(image) => Ok(image),
        Err(_) if allow_cpu_fallback => {
            drop(gpu_scene);
            drop(renderer);
            let expanded = scene.to_scene()?;
            drop(scene);
            render::render(&expanded, options)
        }
        Err(error) => Err(error),
    }
}

/// Generate a thumbnail with normal saved-image preference and selected GPU geometry rendering.
///
/// Raster images and saved document previews retain their existing decoding and
/// fitting behavior; a GPU is initialized only when geometry rendering is needed.
/// `allow_cpu_fallback` has the same meaning as in [`geometry_thumbnail`].
///
/// # Errors
/// Returns the existing preview/import/decoding errors, or errors documented by
/// [`geometry_thumbnail`] when the input needs geometry rendering.
pub fn thumbnail(
    path: &Path,
    options: &RenderOptions,
    gpu: GpuOptions,
    allow_cpu_fallback: bool,
) -> Result<RgbaImage> {
    crate::thumbnail::thumbnail_with_geometry(path, options, |path, options, decoded| {
        geometry_from_source(path, options, gpu, allow_cpu_fallback, decoded)
    })
}
