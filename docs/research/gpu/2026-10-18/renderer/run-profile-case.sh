#!/bin/sh
# Container entry: strict renderer choice; optional GPU never falls back here.
set -u
input=${1:?input path required}
mode=${2:?cpu, shared-cpu or gpu required}
output=${3:?output directory required}
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/start-utc.txt"
/probe/profile_gpu "$input" --mode "$mode" --fresh-process \
  --iterations 3 --size 512 --samples 1 --backend gl \
  --adapter 'UHD Graphics 600' --gpu-memory-mib 256 \
  --report "$output/fresh.json" > "$output/fresh.stdout" 2> "$output/fresh.stderr"
fresh_status=$?
printf '%s\n' "$fresh_status" > "$output/fresh.exit-status"
/probe/profile_gpu "$input" --mode "$mode" \
  --iterations 3 --size 512 --samples 1 --backend gl \
  --adapter 'UHD Graphics 600' --gpu-memory-mib 256 \
  --report "$output/retained.json" > "$output/retained.stdout" 2> "$output/retained.stderr"
retained_status=$?
printf '%s\n' "$retained_status" > "$output/retained.exit-status"
/probe/profile_gpu "$input" --mode "$mode" --reload-scene \
  --iterations 3 --size 512 --samples 1 --backend gl \
  --adapter 'UHD Graphics 600' --gpu-memory-mib 256 \
  --report "$output/reload.json" > "$output/reload.stdout" 2> "$output/reload.stderr"
reload_status=$?
printf '%s\n' "$reload_status" > "$output/reload.exit-status"
sh /probe/capture-memory.sh "$output/container-memory.txt"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/end-utc.txt"
[ "$fresh_status" -eq 0 ] && [ "$retained_status" -eq 0 ] && [ "$reload_status" -eq 0 ]
