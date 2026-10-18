//! Backend selection keeps complete CPU fallback and saved/raster image preference.
#![cfg(all(feature = "gpu", not(target_family = "wasm")))]

use catwalk3d::{
    gpu::{self, GpuOptions},
    render::RenderOptions,
};
use std::path::Path;

/// No backend is initialized, keeping these policy checks independent of host drivers.
fn unavailable() -> GpuOptions {
    GpuOptions {
        backends: wgpu::Backends::empty(),
        ..GpuOptions::default()
    }
}

/// Small reference output with transparent edge coverage.
fn options() -> RenderOptions {
    RenderOptions {
        width: 64,
        height: 48,
        transparent: true,
        supersampling: 2,
        ..RenderOptions::default()
    }
}

#[test]
fn missing_gpu_falls_back_to_the_complete_cpu_geometry() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/models/gear.obj");
    let cpu = catwalk3d::geometry_thumbnail(&path, &options()).unwrap();
    let fallback = gpu::geometry_thumbnail(&path, &options(), unavailable(), true).unwrap();
    assert_eq!(fallback, cpu);
    assert!(gpu::geometry_thumbnail(&path, &options(), unavailable(), false).is_err());
}

#[test]
fn raster_previews_do_not_initialize_a_gpu_even_in_required_mode() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/gear.png");
    let cpu = catwalk3d::thumbnail(&path, &options()).unwrap();
    let selected = gpu::thumbnail(&path, &options(), unavailable(), false).unwrap();
    assert_eq!(selected, cpu);
}

#[test]
fn scalar_tiff_volume_requires_the_selected_gpu_in_required_mode() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formats/tiff_volumes/tif-scalar-asymmetric-volume.tif");
    let error = gpu::thumbnail(&path, &options(), unavailable(), false).unwrap_err();
    assert!(error.to_string().contains("GPU backends"));
}

#[test]
fn scalar_tiff_volume_automatic_fallback_preserves_exact_cpu_pixels_and_png() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formats/tiff_volumes/tif-scalar-asymmetric-volume.tif");
    let expected = catwalk3d::thumbnail(&path, &options()).unwrap();
    let fallback = gpu::thumbnail(&path, &options(), unavailable(), true).unwrap();
    assert_eq!(fallback, expected);
    let mut expected_png = Vec::new();
    let mut fallback_png = Vec::new();
    catwalk3d::render::write_png(&expected, &mut expected_png).unwrap();
    catwalk3d::render::write_png(&fallback, &mut fallback_png).unwrap();
    assert_eq!(fallback_png, expected_png);
}

#[test]
fn raster_tiff_still_bypasses_gpu_initialization_in_required_mode() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formats/images_points/tiff-pattern.tiff");
    let expected = catwalk3d::thumbnail(&path, &options()).unwrap();
    let selected = gpu::thumbnail(&path, &options(), unavailable(), false).unwrap();
    assert_eq!(selected, expected);
}
