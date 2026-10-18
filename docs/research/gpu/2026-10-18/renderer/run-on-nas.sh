#!/bin/sh
# Deliberately refuses an unfinalized bundle. No downloads or production changes.
set -eu
bundle=${1:?new temporary bundle directory required}
stage=${2:-all}
case "$stage" in all|profile|cli) ;; *) printf 'stage must be all, profile or cli\n' >&2; exit 2 ;; esac
if [ ! -f "$bundle/SHA256SUMS" ] || [ ! -f "$bundle/manifest.json" ] \
  || ! grep '"status"[[:space:]]*:[[:space:]]*"finalized"' "$bundle/manifest.json" > /dev/null
then
  printf 'Finalize manifest.json and SHA256SUMS from the actual completed build first.\n' >&2
  exit 2
fi
(cd "$bundle" && sha256sum -c SHA256SUMS) > "$bundle/checksums-verified.txt"
docker_bin=${DOCKER_BIN:-/usr/local/bin/docker}
image=catwalk-gpu-probe:20261018
expected_image=sha256:ffabbe46b4a5b81eb9cd73334047454bd6420a5cc668f43d6b9920bc1ae070dd
actual_image=$("$docker_bin" image inspect "$image" --format '{{.Id}}')
if [ "$actual_image" != "$expected_image" ]; then
  printf 'Unexpected existing runtime image: %s\n' "$actual_image" >&2
  exit 2
fi
render_node=/dev/dri/renderD128
render_group=$(stat -c %g "$render_node")
results="$bundle/results"
mkdir -p "$results"
overall_failed=0
if { [ "$stage" != cli ] && [ -e "$results/profile" ]; } \
  || { [ "$stage" != profile ] && [ -e "$results/cli" ]; }
then
  printf 'Requested results already exist; preserve them and use a fresh bundle for reruns.\n' >&2
  exit 2
fi
bundle_real=$(CDPATH= cd "$bundle" && pwd -P)
bundle_mount=$(awk -v bundle="$bundle_real" '
  { point=$2; if (point == "/" || bundle == point || index(bundle, point "/") == 1) {
      if (length(point) > longest) { longest=length(point); record=$0 }
    }
  }
  END { print record }
' /proc/mounts)
{
  printf 'bundle=%s\n' "$bundle_real"
  df -Pk "$bundle_real"
  printf '\ncontaining mount from /proc/mounts:\n%s\n' "$bundle_mount"
} > "$results/bundle-filesystem-$stage.txt"
bundle_fstype=$(printf '%s\n' "$bundle_mount" | awk '{ print $3 }')
case "$bundle_fstype" in
  tmpfs|ramfs)
    printf 'Stage the NAS bundle/test payloads on disk; current filesystem is %s.\n' "$bundle_fstype" >&2
    exit 2
    ;;
esac
cp "$bundle/manifest.json" "$results/manifest-$stage.json"
cp "$bundle/SHA256SUMS" "$results/SHA256SUMS-$stage"
cp "$bundle/checksums-verified.txt" "$results/checksums-verified-$stage.txt"
(cd "$bundle" && sha256sum bin/catwalk bin/profile_gpu inputs/* policy.json scripts/* run-on-nas.sh manifest.json) \
  > "$results/deployed-sha256-$stage.txt"
{
  date -u '+%Y-%m-%dT%H:%M:%SZ'
  uname -a
  id
  printf 'render_node=%s render_group=%s\n' "$render_node" "$render_group"
  printf 'image=%s\n' "$actual_image"
  cat /proc/meminfo
} > "$results/host-before-$stage.txt"
run_case() {
  kind=$1
  input=$2
  mode=$3
  case_output="$results/$kind/${input%.*}/$mode"
  mkdir -p "$case_output"
  if [ "$kind" = profile ]; then runner=run-profile-case.sh; timeout=120s; else runner=run-cli-case.sh; timeout=600s; fi
  set +e
  "$docker_bin" run --rm --network none --read-only \
    --user "$(id -u):$(id -g)" --group-add "$render_group" \
    --device "$render_node:$render_node:rw" --cpuset-cpus 3 \
    --memory=512m --memory-swap=512m \
    --cap-drop ALL --security-opt no-new-privileges --tmpfs /tmp:rw,size=32m \
    -e EGL_PLATFORM=surfaceless -e DISPLAY= -e WAYLAND_DISPLAY= \
    -e XDG_CACHE_HOME=/tmp/cache -e LIBGL_ALWAYS_SOFTWARE=0 \
    --mount "type=bind,src=$bundle/bin,dst=/probe/bin,readonly" \
    --mount "type=bind,src=$bundle/bin/profile_gpu,dst=/probe/profile_gpu,readonly" \
    --mount "type=bind,src=$bundle/bin/catwalk,dst=/probe/catwalk,readonly" \
    --mount "type=bind,src=$bundle/scripts,dst=/probe/scripts,readonly" \
    --mount "type=bind,src=$bundle/scripts/$runner,dst=/probe/$runner,readonly" \
    --mount "type=bind,src=$bundle/scripts/capture-memory.sh,dst=/probe/capture-memory.sh,readonly" \
    --mount "type=bind,src=$bundle/inputs,dst=/inputs,readonly" \
    --mount "type=bind,src=$case_output,dst=/output" \
    --entrypoint /usr/bin/timeout "$image" \
    --kill-after=5s "$timeout" /bin/sh "/probe/$runner" "/inputs/$input" "$mode" /output \
    > "$case_output/container.stdout" 2> "$case_output/container.stderr"
  status=$?
  set -e
  printf '%s\n' "$status" > "$case_output/container.exit-status"
  printf '%s %s %s: exit %s\n' "$kind" "$input" "$mode" "$status"
  [ "$status" -eq 0 ] || overall_failed=1
}
if [ "$stage" != cli ]; then
  for input in gear.obj ap242-tessellated.stp synthetic-repeated.step
  do
    for mode in cpu shared-cpu gpu
    do run_case profile "$input" "$mode"; done
  done
fi
if [ "$stage" != profile ]; then
  for input in gear.obj ap242-tessellated.stp synthetic-repeated.step
  do run_case cli "$input" supported; done
  for input in jpeg-textured.3mf alpha-modes.glb
  do run_case cli "$input" unsupported; done
fi
{
  date -u '+%Y-%m-%dT%H:%M:%SZ'
  cat /proc/meminfo
} > "$results/host-after-$stage.txt"
[ "$overall_failed" -eq 0 ]
