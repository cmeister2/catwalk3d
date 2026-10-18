//! Measure CPU geometry-thumbnail phases without changing the thumbnail pipeline.
//!
//! Run `cargo run --release --example profile_thumbnail -- MODEL [REPORT.json]`
//! with optional `--size 512 --samples 2 --iterations 5 --png LAST.png`.
//! `--fresh-process` runs each job in a child process and measures launch to exit;
//! `--reuse-scene` instead imports once and measures repeated renders in one process.
//! Filesystem caches are never flushed. Import and geometry processing are one
//! measured phase because the existing public importer performs both operations.

use catwalk3d::{
    Result, formats,
    render::{self, RenderOptions},
    scene::Scene,
};
use serde_json::{Value, json};
use std::{
    env,
    ffi::OsString,
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

/// Parsed profiling controls; the default path reloads geometry for every job.
struct Options {
    /// Input geometry file, including any relative material resources.
    model: PathBuf,
    /// Optional JSON destination; stdout is used when absent.
    report: Option<PathBuf>,
    /// Optional destination for the final encoded PNG.
    png: Option<PathBuf>,
    /// CPU rendering settings shared by all jobs.
    render: RenderOptions,
    /// Number of measured jobs.
    iterations: usize,
    /// Retain an imported scene to isolate repeated rendering and encoding.
    reuse_scene: bool,
    /// Launch a new process per job, resetting process memory high-water marks.
    fresh_process: bool,
}

/// Linux process accounting observed at one boundary, with unavailable fields absent.
#[derive(Clone, Copy)]
struct Resources {
    /// Current resident set in KiB, sampled rather than continuously monitored.
    rss_kib: Option<u64>,
    /// Lifetime resident-set high-water mark in KiB; cumulative within a process.
    hwm_kib: Option<u64>,
    /// Lifetime user-mode CPU clock ticks from Linux procfs.
    user_ticks: Option<u64>,
    /// Lifetime kernel-mode CPU clock ticks from Linux procfs.
    system_ticks: Option<u64>,
}

/// Profile selected jobs, writing one machine-readable JSON report.
fn main() -> Result<()> {
    let entry = Instant::now();
    let Some(options) = parse_options()? else {
        return Ok(());
    };
    let report = if options.fresh_process {
        profile_children(&options)?
    } else {
        profile_in_process(&options, entry)?
    };
    let mut encoded = serde_json::to_vec_pretty(&report)?;
    encoded.push(b'\n');
    if let Some(path) = options.report {
        fs::write(path, encoded)?;
    } else {
        use std::io::Write as _;
        std::io::stdout().lock().write_all(&encoded)?;
    }
    Ok(())
}

/// Parse simple positional paths and profiling flags without adding dependencies.
fn parse_options() -> Result<Option<Options>> {
    let mut model = None;
    let mut report = None;
    let mut png = None;
    let mut render = RenderOptions::default();
    let mut iterations = 1;
    let mut reuse_scene = false;
    let mut fresh_process = false;
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!(
                    "Usage: profile_thumbnail MODEL [REPORT.json] [--size N] \
                     [--samples N] [--iterations N] [--png LAST.png] \
                     [--reuse-scene | --fresh-process]\n\
                     Always imports geometry; saved document previews are bypassed.\n\
                     Fresh processes do not imply cold filesystem caches.\n\
                     Linux RSS is sampled; VmHWM is cumulative per process."
                );
                return Ok(None);
            }
            Some("--size") => {
                let size = next_number(&mut args, "--size")?;
                render.width = size;
                render.height = size;
            }
            Some("--samples") => render.supersampling = next_number(&mut args, "--samples")?,
            Some("--iterations") => iterations = next_number(&mut args, "--iterations")?,
            Some("--png") => png = Some(PathBuf::from(args.next().ok_or("--png needs a path")?)),
            Some("--reuse-scene") => reuse_scene = true,
            Some("--fresh-process") => fresh_process = true,
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("unknown flag: {flag}").into());
            }
            _ if model.is_none() => model = Some(PathBuf::from(arg)),
            _ if report.is_none() => report = Some(PathBuf::from(arg)),
            _ => return Err("unexpected positional argument".into()),
        }
    }
    if iterations == 0 {
        return Err("--iterations must be positive".into());
    }
    if render.width == 0 || render.width > 8192 || !(1..=4).contains(&render.supersampling) {
        return Err("--size must be 1..=8192 and --samples must be 1..=4".into());
    }
    if reuse_scene && fresh_process {
        return Err("--reuse-scene and --fresh-process describe different benchmark modes".into());
    }
    Ok(Some(Options {
        model: model.ok_or("a geometry input path is required; see --help")?,
        report,
        png,
        render,
        iterations,
        reuse_scene,
        fresh_process,
    }))
}

/// Read one numeric flag argument, reporting missing, non-UTF-8 or invalid values.
fn next_number<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<T> {
    args.next()
        .and_then(|arg| arg.into_string().ok())
        .and_then(|arg| arg.parse().ok())
        .ok_or_else(|| format!("{flag} requires a valid number").into())
}

/// Profile child lifetimes, retaining each child's independent phase report.
fn profile_children(options: &Options) -> Result<Value> {
    let mut jobs = Vec::with_capacity(options.iterations);
    for iteration in 0..options.iterations {
        let mut command = Command::new(env::current_exe()?);
        command
            .arg(&options.model)
            .arg("--size")
            .arg(options.render.width.to_string())
            .arg("--samples")
            .arg(options.render.supersampling.to_string());
        if iteration + 1 == options.iterations
            && let Some(path) = &options.png
        {
            command.arg("--png").arg(path);
        }
        let start = Instant::now();
        let output = command.output()?;
        let wall = milliseconds(start.elapsed());
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
            "subprocess_launch_to_exit_ms": wall,
            "report": serde_json::from_slice::<Value>(&output.stdout)?,
        }));
    }
    Ok(json!({
        "schema_version": 1,
        "mode": "fresh_process_per_job",
        "input": options.model.to_string_lossy(),
        "notes": [
            "Each child has a new process and independent VmHWM; filesystem caches are not flushed.",
            "Launch-to-exit includes child startup, procfs/getconf accounting and JSON serialization.",
            "This geometry-only profiler bypasses saved previews and is not the production CLI.",
            "Use an external process monitor for production CLI peak RSS and whole-job CPU accounting."
        ],
        "jobs": jobs,
    }))
}

/// Profile repeated CPU work, either reloading geometry or retaining one scene.
fn profile_in_process(options: &Options, entry: Instant) -> Result<Value> {
    let ticks_per_second = clock_ticks_per_second();
    let mut setup = Value::Null;
    let retained = if options.reuse_scene {
        let before = resources();
        let start = Instant::now();
        let scene = formats::load(&options.model)?;
        let wall = start.elapsed();
        let after = resources();
        setup = phase(wall, before, after, ticks_per_second);
        Some(scene)
    } else {
        None
    };
    let xy = matches!(
        formats::extension(&options.model)?.as_str(),
        "lbrn" | "lbrn2"
    );
    let mut jobs = Vec::with_capacity(options.iterations);
    for iteration in 0..options.iterations {
        let job_start = Instant::now();
        let before = resources();
        let mut loaded = None;
        let import = if retained.is_none() {
            let start = Instant::now();
            loaded = Some(formats::load(&options.model)?);
            let wall = start.elapsed();
            phase(wall, before, resources(), ticks_per_second)
        } else {
            Value::Null
        };
        let scene = retained
            .as_ref()
            .or(loaded.as_ref())
            .ok_or("missing scene")?;
        let counts = scene_counts(scene);
        let before_render = resources();
        let start = Instant::now();
        let image = if xy {
            render::render_xy(scene, &options.render)?
        } else {
            render::render(scene, &options.render)?
        };
        let render_wall = start.elapsed();
        let after_render = resources();
        let start = Instant::now();
        let mut encoded = Vec::new();
        render::write_png(&image, &mut encoded)?;
        let encode_wall = start.elapsed();
        let after_encode = resources();
        let png_bytes = encoded.len();
        let mut write = Value::Null;
        if iteration + 1 == options.iterations
            && let Some(path) = &options.png
        {
            let start = Instant::now();
            fs::write(path, &encoded)?;
            write = phase(start.elapsed(), after_encode, resources(), ticks_per_second);
        }
        std::hint::black_box(&encoded);
        drop(encoded);
        drop(image);
        drop(loaded);
        let job_wall = job_start.elapsed();
        let after = resources();
        jobs.push(json!({
            "iteration": iteration,
            "scene": counts,
            "load_and_geometry": import,
            "render": phase(render_wall, before_render, after_render, ticks_per_second),
            "png_encode_to_memory": phase(encode_wall, after_render, after_encode, ticks_per_second),
            "png_write": write,
            "png_bytes": png_bytes,
            "job_with_cleanup_and_accounting": phase(job_wall, before, after, ticks_per_second),
        }));
    }
    Ok(json!({
        "schema_version": 1,
        "mode": if options.reuse_scene { "same_process_reuse_scene" } else { "same_process_reload_scene" },
        "input": options.model.to_string_lossy(),
        "width": options.render.width,
        "height": options.render.height,
        "supersampling": options.render.supersampling,
        "cpu_clock_ticks_per_second": ticks_per_second,
        "setup_load_and_geometry": setup,
        "jobs": jobs,
        "process_entry_to_report_ms": milliseconds(entry.elapsed()),
        "notes": [
            "Import includes file reads, decoding, geometry processing, tessellation and validation; these are not separately instrumented.",
            "Rendering includes scene validation, bounds/projection, rasterization, compositing and output resizing.",
            "PNG encoding targets memory; optional final PNG writing has a separate phase and does not fsync.",
            "RSS values are endpoint samples, not phase peaks. VmHWM is cumulative for the entire process and includes setup/earlier jobs.",
            "CPU values are procfs clock-tick deltas; null means unsupported/unavailable and short phases may round to zero.",
            "Repeated jobs retain filesystem caches and allocator state. No job is claimed to have cold filesystem caches.",
            "Phase wall timers exclude boundary resource sampling; job/entry timers include instrumentation and cleanup.",
            "Entry-to-report excludes executable startup, JSON serialization/writing and final retained-scene destruction.",
            "Process RSS excludes kernel page cache and is not a measure of total system or GPU memory.",
            "This geometry-only profiler bypasses saved previews and does not reproduce production CLI argument/output overhead."
        ],
    }))
}

/// Describe stored primitives and appearance records without estimating heap size.
fn scene_counts(scene: &Scene) -> Value {
    json!({
        "triangles": scene.triangles.len(),
        "points": scene.points.len(),
        "lines": scene.lines.len(),
        "triangle_appearances": scene.appearances.len(),
    })
}

/// Convert a duration to fractional milliseconds.
fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Return resource observations and CPU deltas for one measured wall interval.
fn phase(wall: Duration, before: Resources, after: Resources, ticks: Option<u64>) -> Value {
    json!({
        "wall_ms": milliseconds(wall),
        "user_cpu_ms": cpu_delta(before.user_ticks, after.user_ticks, ticks),
        "system_cpu_ms": cpu_delta(before.system_ticks, after.system_ticks, ticks),
        "before": resource_values(before),
        "after": resource_values(after),
    })
}

/// Convert process CPU tick deltas to milliseconds; never assume a platform tick rate.
#[allow(clippy::cast_precision_loss)]
fn cpu_delta(before: Option<u64>, after: Option<u64>, ticks: Option<u64>) -> Option<f64> {
    let elapsed = after?.checked_sub(before?)?;
    let rate = ticks?;
    (rate > 0).then(|| elapsed as f64 * 1000.0 / rate as f64)
}

/// Encode sampled memory and cumulative CPU values, preserving unavailable fields.
fn resource_values(value: Resources) -> Value {
    json!({
        "rss_kib": value.rss_kib,
        "process_lifetime_hwm_kib": value.hwm_kib,
        "user_cpu_ticks_total": value.user_ticks,
        "system_cpu_ticks_total": value.system_ticks,
    })
}

/// Read Linux memory and CPU accounting without failing a job on other systems.
fn resources() -> Resources {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib = |name: &str| {
        status.lines().find_map(|line| {
            let value = line.strip_prefix(name)?;
            let mut fields = value.split_whitespace();
            let number = fields.next()?.parse().ok()?;
            (fields.next()? == "kB").then_some(number)
        })
    };
    // The comm field can contain spaces and parentheses; fields after its final
    // closing parenthesis start at kernel stat field 3 (process state).
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
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

/// Ask the operating system for procfs CPU tick frequency, or leave CPU time absent.
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
