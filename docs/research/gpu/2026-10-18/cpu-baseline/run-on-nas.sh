#!/bin/sh
# Run only in a temporary directory containing the explicit profiler bundle.
# Does not build/pull images, access a GPU, or modify application containers.
set -eu
bundle=${1:?temporary bundle directory required}
docker_bin=${DOCKER_BIN:-/usr/local/bin/docker}
image=catwalk-gpu-probe:20261018
results="$bundle/results"
mkdir -p "$results"
{
  date -u '+%Y-%m-%dT%H:%M:%SZ'
  uname -a
  id
  "$docker_bin" image inspect "$image" --format '{{.Id}} {{.Os}} {{.Architecture}}'
  cat /proc/meminfo
} > "$results/host-before.txt"
sha256sum "$bundle/profile_thumbnail" "$bundle"/inputs/* > "$results/deployed-sha256.txt"
for input in gear.obj repeated-assembly.step ap242-tessellated.stp jpeg-textured.3mf alpha-modes.glb
do
  output="$results/${input%.*}"
  mkdir -p "$output"
  set +e
  "$docker_bin" run --rm --network none --read-only \
    --user "$(id -u):$(id -g)" --cpuset-cpus 3 \
    --memory=512m --memory-swap=512m \
    --cap-drop ALL --security-opt no-new-privileges --tmpfs /tmp:rw,size=32m \
    --mount "type=bind,src=$bundle/profile_thumbnail,dst=/probe/profile_thumbnail,readonly" \
    --mount "type=bind,src=$bundle/run-one.sh,dst=/probe/run-one.sh,readonly" \
    --mount "type=bind,src=$bundle/inputs,dst=/inputs,readonly" \
    --mount "type=bind,src=$output,dst=/output" \
    --entrypoint /usr/bin/timeout "$image" \
    --kill-after=5s 120s /bin/sh /probe/run-one.sh "/inputs/$input" /output \
    > "$output/container.stdout" 2> "$output/container.stderr"
  status=$?
  set -e
  printf '%s\n' "$status" > "$output/container.exit-status"
  printf '%s: exit %s\n' "$input" "$status"
done
{
  date -u '+%Y-%m-%dT%H:%M:%SZ'
  cat /proc/meminfo
} > "$results/host-after.txt"
