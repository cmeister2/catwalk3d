#!/bin/sh
# Runs inside the isolated container. The outer timeout covers both modes.
set -u
input=${1:?input path required}
output=${2:?output directory required}
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/start-utc.txt"
ldd --version > "$output/libc.txt" 2>&1
/probe/profile_thumbnail "$input" "$output/fresh.json" \
  --fresh-process --iterations 3 --size 512 --samples 1 \
  > "$output/fresh.stdout" 2> "$output/fresh.stderr"
fresh_status=$?
printf '%s\n' "$fresh_status" > "$output/fresh.exit-status"
/probe/profile_thumbnail "$input" "$output/reuse.json" \
  --reuse-scene --iterations 3 --size 512 --samples 1 \
  > "$output/reuse.stdout" 2> "$output/reuse.stderr"
reuse_status=$?
printf '%s\n' "$reuse_status" > "$output/reuse.exit-status"
# These may be absent, depending on cgroup version and mount visibility. Their
# scope is the container and includes page cache/kernel charges, unlike VmHWM.
for stat in \
  /sys/fs/cgroup/memory/memory.max_usage_in_bytes \
  /sys/fs/cgroup/memory/memory.limit_in_bytes \
  /sys/fs/cgroup/memory/memory.memsw.limit_in_bytes \
  /sys/fs/cgroup/memory/memory.failcnt \
  /sys/fs/cgroup/memory/memory.stat \
  /sys/fs/cgroup/memory.peak \
  /sys/fs/cgroup/memory.max \
  /sys/fs/cgroup/memory.events
do
  if [ -r "$stat" ]; then
    printf '\n%s\n' "$stat" >> "$output/container-memory.txt"
    cat "$stat" >> "$output/container-memory.txt"
  fi
done
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/end-utc.txt"
[ "$fresh_status" -eq 0 ] && [ "$reuse_status" -eq 0 ]
