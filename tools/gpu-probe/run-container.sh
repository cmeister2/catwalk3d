#!/bin/sh
# Build with cargo build --release --locked. Runtime image needs EGL/GLES drivers.
# NAS example: DOCKER_BIN=/usr/local/bin/docker GPU_PROBE_CPU=3 sh run-container.sh
set -eu
probe_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
docker_bin=${DOCKER_BIN:-docker}
render_node=${GPU_RENDER_NODE:-/dev/dri/renderD128}
image=${GPU_PROBE_IMAGE:-catwalk-gpu-probe:20261018}
binary=${GPU_PROBE_BIN:-"$probe_dir/target/release/catwalk-wgpu-probe"}
if [ ! -f "$binary" ]; then binary="$probe_dir/catwalk-wgpu-probe"; fi
output_dir=${GPU_PROBE_OUTPUT_DIR:-"$probe_dir/output"}
mkdir -p "$output_dir"
if [ -n "${GPU_PROBE_CPU:-}" ]; then
  set -- --cpuset-cpus "$GPU_PROBE_CPU"
else
  set --
fi
exec "$docker_bin" run --rm --network none --read-only \
  --user "$(id -u):$(id -g)" --group-add "$(stat -c %g "$render_node")" \
  --device "$render_node:$render_node:rw" "$@" \
  --memory="${GPU_PROBE_MEMORY:-256m}" --memory-swap="${GPU_PROBE_MEMORY:-256m}" \
  --cap-drop ALL --security-opt no-new-privileges --tmpfs /tmp:rw,size=32m \
  -e EGL_PLATFORM=surfaceless -e DISPLAY= -e WAYLAND_DISPLAY= \
  -e XDG_CACHE_HOME=/tmp/cache -e LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-0}" \
  --mount "type=bind,src=$binary,dst=/probe/catwalk-wgpu-probe,readonly" \
  --mount "type=bind,src=$output_dir,dst=/output" \
  --entrypoint /probe/catwalk-wgpu-probe "$image" \
  --backend gl --require-intel --output-dir /output --iterations 16
