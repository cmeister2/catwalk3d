#!/bin/sh
# Container entry: correctness matrix, separate from timing benchmarks.
set -u
input=${1:?input path required}
support=${2:?supported or unsupported required}
output=${3:?output directory required}
failed=0
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/start-utc.txt"
printf 'renderer\tsamples\tbackground\texit_status\texpectation_met\n' > "$output/status.tsv"
for samples in 1 2
do
  for background in opaque transparent
  do
    if [ "$background" = transparent ]; then set -- --transparent; else set --; fi
    if [ "$support" = unsupported ]; then renderers='cpu auto gpu'; else renderers='cpu gpu'; fi
    for renderer in $renderers
    do
      stem="$renderer-s$samples-$background"
      /usr/bin/timeout --kill-after=5s 120s /probe/catwalk "$input" \
        --output "$output/$stem.png" --render-geometry --renderer "$renderer" \
        --size 512 --samples "$samples" --gpu-backend gl \
        --gpu-adapter 'UHD Graphics 600' --gpu-memory-mib 256 "$@" \
        > "$output/$stem.stdout" 2> "$output/$stem.stderr"
      status=$?
      met=1
      if [ "$support" = unsupported ] && [ "$renderer" = gpu ]; then
        if [ "$status" -eq 0 ] || [ "$status" -eq 124 ] || [ "$status" -eq 137 ] \
          || [ -e "$output/$stem.png" ] \
          || ! grep -F 'supports opaque untextured materials' "$output/$stem.stderr" > /dev/null
        then met=0; fi
      elif [ "$status" -ne 0 ] || [ ! -s "$output/$stem.png" ]; then
        met=0
      fi
      [ "$met" -eq 1 ] || failed=1
      printf '%s\t%s\t%s\t%s\t%s\n' "$renderer" "$samples" "$background" "$status" "$met" >> "$output/status.tsv"
    done
  done
done
sh /probe/capture-memory.sh "$output/container-memory.txt"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$output/end-utc.txt"
[ "$failed" -eq 0 ]
