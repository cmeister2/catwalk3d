set -eu
task_scratch=/volume1/homes/max/.catwalk-gpu-20261018.n0HIq9
test -d "$task_scratch"
test ! -e "$task_scratch/earlier-tmp-scratch"
printf 'Before relocation, UTC: '
date -u
cat /proc/meminfo
for source in /tmp/catwalk-cpu-baseline-20261018.EOdA5f /tmp/catwalk-wgpu-probe-20261018 /tmp/catwalk-gpu-probe-1792349138088; do
  test -d "$source"
  test ! -L "$source"
  du -sk "$source"
done
mkdir "$task_scratch/earlier-tmp-scratch"
mv /tmp/catwalk-cpu-baseline-20261018.EOdA5f "$task_scratch/earlier-tmp-scratch/"
mv /tmp/catwalk-wgpu-probe-20261018 "$task_scratch/earlier-tmp-scratch/"
mv /tmp/catwalk-gpu-probe-1792349138088 "$task_scratch/earlier-tmp-scratch/"
printf 'After relocation, UTC: '
date -u
du -sk "$task_scratch/earlier-tmp-scratch/"*
cat /proc/meminfo
/usr/local/bin/docker ps --filter name=manyfold-manyfold-1 --format '{{.Names}} {{.Status}}'
