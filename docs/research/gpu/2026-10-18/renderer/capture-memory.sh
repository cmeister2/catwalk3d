#!/bin/sh
# These are cgroup charges, not complete process/GPU/system memory consumption.
set -eu
output=${1:?output path required}
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
    printf '\n%s\n' "$stat" >> "$output"
    cat "$stat" >> "$output"
  fi
done
printf '\nproc_self_cpus_allowed_list\n' >> "$output"
sed -n 's/^Cpus_allowed_list:[[:space:]]*//p' /proc/self/status >> "$output"
