"""Record a completed native build and the exact source inputs it used."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

REPO = Path('/tmp/catwalk-wgpu-worktree')

def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        while block := f.read(1024 * 1024):
            h.update(block)
    return h.hexdigest()

def capture_source():
    names = subprocess.check_output(['git', 'ls-files', 'src', 'Cargo.toml', 'Cargo.lock', 'README.md', 'examples/profile_gpu.rs', 'examples/models/gear.obj'], cwd=REPO, text=True).splitlines()
    return {name: digest(REPO / name) for name in names}

def main():
    p = argparse.ArgumentParser()
    p.add_argument('platform', choices=['linux', 'windows'])
    p.add_argument('--windows-target', type=Path, default=Path('/tmp/catwalk-windows-target'))
    args = p.parse_args()
    prefix = Path('/tmp/catwalk-final-build-' + args.platform)
    before = capture_source()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    env = os.environ.copy()
    if args.platform == 'linux':
        command = ['cargo', 'build', '--release', '--features', 'gpu', '--locked', '--offline', '--bin', 'catwalk', '--example', 'profile_gpu']
        env['CARGO_TARGET_DIR'] = '/home/max/code/github/catwalk/target'
        compiler = ['rustc', '-Vv']
        executable = Path(env['CARGO_TARGET_DIR']) / 'release/catwalk'
        profiler = executable.parent / 'examples/profile_gpu'
    else:
        windows_target = args.windows_target.resolve()
        windows_target_native = subprocess.check_output(['wslpath', '-w', str(windows_target)], text=True).strip()
        command = ['/mnt/c/Users/jungl/.cargo/bin/cargo.exe', '+1.99.0', 'build', '--release', '--locked', '--features', 'gpu', '--bin', 'catwalk', '--example', 'profile_gpu', '--target-dir', windows_target_native]
        compiler = ['/mnt/c/Users/jungl/.cargo/bin/rustc.exe', '+1.99.0', '-Vv']
        executable = windows_target / 'release/catwalk.exe'
        profiler = executable.parent / 'examples/profile_gpu.exe'
    compiler_text = subprocess.check_output(compiler, cwd=REPO, text=True)
    started = time.time()
    with prefix.with_suffix('.log').open('wb') as log:
        status = subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
    if status:
        raise SystemExit(f'Build failed ({status}); see {prefix}.log')
    after = capture_source()
    if before != after:
        raise SystemExit('Source inputs changed during build; no completed metadata emitted')
    metadata = {
        'source_provenance': {'git_commit': revision, 'source_files_sha256': after, 'source_unchanged_during_build': True},
        'cargo_lock_sha256': digest(REPO / 'Cargo.lock'),
        'compiler': compiler_text,
        'command': command,
        'environment': {'CARGO_TARGET_DIR': env.get('CARGO_TARGET_DIR')},
        'platform': args.platform,
        'target_directory_wsl': str(executable.parent.parent),
        'target_directory_native': windows_target_native if args.platform == 'windows' else None,
        'build_started_unix': started,
        'build_finished_unix': time.time(),
        'catwalk_binary_sha256': digest(executable),
        'profile_gpu_binary_sha256': digest(profiler),
        'profile_gpu_source_sha256': digest(REPO / 'examples/profile_gpu.rs'),
        'build_log_sha256': digest(prefix.with_suffix('.log')),
    }
    prefix.with_suffix('.json').write_text(json.dumps(metadata, indent=2) + '\n')
    print(prefix.with_suffix('.json'))

if __name__ == '__main__':
    main()
