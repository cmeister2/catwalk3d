#!/bin/sh
set -eu
directory=${1:?scratch directory required}
test -f "$directory/tests"
test "$(sha256sum "$directory/tests" | cut -d ' ' -f 1)" = 7796da68cda2f1663cb7cd82c717c649fc1e83528717d781bf7890db58a4d337
docker_bin=/usr/local/bin/docker
image=catwalk-gpu-probe:20261018
test "$("$docker_bin" image inspect "$image" --format '{{.Id}}')" = sha256:ffabbe46b4a5b81eb9cd73334047454bd6420a5cc668f43d6b9920bc1ae070dd
render_group=$(stat -c %g /dev/dri/renderD128)
mkdir "$directory/output"
cat /proc/meminfo > "$directory/host-memory-before.txt"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$directory/start-utc.txt"
set +e
"$docker_bin" run --rm --network none --read-only \
  --user "$(id -u):$(id -g)" --group-add "$render_group" \
  --device /dev/dri/renderD128:/dev/dri/renderD128:rw --cpuset-cpus 3 \
  --memory=512m --memory-swap=512m --cap-drop ALL \
  --security-opt no-new-privileges --tmpfs /tmp:rw,size=32m \
  -e EGL_PLATFORM=surfaceless -e DISPLAY= -e WAYLAND_DISPLAY= \
  -e XDG_CACHE_HOME=/tmp/cache -e LIBGL_ALWAYS_SOFTWARE=0 \
  -e CATWALK_GPU_TEST_BACKEND=gl -e CATWALK_REQUIRE_GPU_TEST_ADAPTER=1 \
  -e CATWALK_REQUIRE_HARDWARE_GPU_TEST=1 -e 'CATWALK_GPU_TEST_ADAPTER=UHD Graphics 600' \
  --mount "type=bind,src=$directory/tests,dst=/probe/tests,readonly" \
  --mount "type=bind,src=$directory/output,dst=/output" \
  --entrypoint /bin/sh "$image" -c '
    /usr/bin/timeout --kill-after=5s 120s /probe/tests gpu::tests:: --nocapture --test-threads=1
    result=$?
    for stat in memory.max_usage_in_bytes memory.limit_in_bytes memory.failcnt; do
      if [ -r /sys/fs/cgroup/memory/$stat ]; then
        printf "%s=" "$stat" >> /output/cgroup.txt
        cat /sys/fs/cgroup/memory/$stat >> /output/cgroup.txt
      fi
    done
    exit "$result"
  ' > "$directory/stdout.txt" 2> "$directory/stderr.txt"
status=$?
set -e
printf '%s\n' "$status" > "$directory/exit-status.txt"
cat /proc/meminfo > "$directory/host-memory-after.txt"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$directory/end-utc.txt"
cat "$directory/stdout.txt" "$directory/stderr.txt"
exit "$status"
