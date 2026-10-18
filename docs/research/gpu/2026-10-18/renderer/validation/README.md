# Validation evidence

These records cover code compatibility, package verification and reproducibility
of archived measurements. They do not qualify production GPU image output: all
12 supported NAS pairs and all 24 native Windows GPU pairs failed the unchanged
strict image gate. See the
[NAS recommendation](../nas-recommendation.md) and
[archived comparisons](../nas-results/comparisons.json).

Source versions differ where recorded: the NAS runs and GPU regression artifacts
identify `6e00d13`; the final GPU Clippy check followed the test-only `ec68f04`
cleanup. The full optional GPU CI run used the unchanged runtime source plus
the later packaging/documentation checkpoint recorded in its metadata. The final
package excludes GPU research artifacts while retaining them in Git. Consult each
build/source manifest rather than assuming all checks share a source revision.

| Check | Recorded result | Evidence and scope |
| --- | --- | --- |
| Default CPU, all targets | 941 passed; no failures or ignored tests | [Full log](default-tests.log), including the complete format snapshot corpus, integration tests and default examples. |
| Full optional GPU CI test command | 567 passed; no failures, ignored or filtered tests | [Full log](gpu-ci-tests.log), [source/environment metadata](gpu-ci-tests-metadata.json): all 550 library tests plus 17 dispatch/shared-scene integrations, with a required software GL adapter. This includes the CPU library tests under the GPU feature. |
| WASI library without default features | Passed | [Full log](wasi-check.log); Clang plus the existing web builder's `-ffreestanding` flag. |
| GPU software GL library tests | 16 passed, including 18 image cases | [Metadata and commands](../depth-precision/post-guard-metadata.json), [test log](../depth-precision/post-guard-software-gl.log); software correctness only. |
| GPU-feature CPU profiling parity | 1 passed | [Exact-output parity log](../depth-precision/post-guard-cpu-profile.log). |
| GPU dispatch/shared-scene integrations | 17 passed | [Log](../depth-precision/post-guard-integrations.log): 5 dispatch tests, 7 indexed/instanced tests, 5 shared STEP tests. |
| NAS hardware GL library tests | 16 passed, including 18 image cases | [Output](../nas-correctness-6e00d13/stdout.txt), [exit status](../nas-correctness-6e00d13/exit-status.txt), [run script](../nas-correctness-6e00d13/run.sh). These cases do not replace the failed full CLI image matrix. |
| GPU Clippy, all targets | Passed, warnings denied | Coordinating agent directly observed session `29812` after the test-only cleanup; the complete command environment and raw log were not retained. |
| GPU private-item documentation | Passed, warnings denied | Coordinating agent directly observed session `25299` on `6e00d13`; the complete command environment and raw log were not retained. |
| Final Cargo package verification | Passed | [Log](final-package-verified.log): Cargo built the final unpacked published crate successfully. |
| Final unpacked crate library tests | 533 passed; no failures or ignored tests | [Full log](final-package-unpacked-lib-tests.log); default CPU library tests from the final generated package, not the working checkout. |
| Final package archive policy | Passed | [Policy result](final-package-policy.json), [contents](final-package-contents.txt): 1,250,270 bytes, 163 entries, no unexpected/forbidden/research paths; all 122 packaged source files match the worktree and the 10,000,000-byte exclusive limit is unchanged. |
| Archived NAS profile revalidation | 27/27 valid experiment reports, 81 jobs | [Log](archive-profile-validation.log); all reconstructed metric vectors matched the archived summary. |
| Archived NAS image revalidation | Expected failure: 8/20 passed | [Log](archive-image-comparison-validation.log); all interior metrics matched the archived comparison, including the 12 GPU maximum-error failures. |

## Recorded CPU, WASI and package commands

The following checks ran in the isolated implementation worktree with
`CARGO_TARGET_DIR=/home/max/code/github/catwalk/target` and
`CARGO_BUILD_JOBS=2`:

```sh
cargo test --all-targets --locked --offline
CC_wasm32_wasip1=clang CFLAGS_wasm32_wasip1=-ffreestanding cargo check --target wasm32-wasip1 --lib --no-default-features --locked --offline
cargo package --allow-dirty --locked --offline
```

The full optional GPU CI test command ran with the same shared target and two
build workers. The recorded command includes `--offline` for the local cache:

```sh
env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_RUNTIME_DIR \
  EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1 CATWALK_REQUIRE_GPU_TEST_ADAPTER=1 \
  cargo test --features gpu --locked --offline --lib --test gpu_dispatch --test instanced --test step_instanced
```

Optional hardware/backend/adapter test selectors were cleared to retain the CI
default GL behavior. The [metadata](gpu-ci-tests-metadata.json) records all
compiled source hashes before the run and confirms that none changed during it.
This unfiltered software correctness pass does not change the recorded failures
under the strict hardware image policy.

The CI-style unpacked-crate test command uses the shared target explicitly:

```sh
CARGO_BUILD_JOBS=2 cargo test --manifest-path /home/max/code/github/catwalk/target/package/catwalk3d-0.1.0/Cargo.toml --lib --locked --target-dir /home/max/code/github/catwalk/target --offline
```

The coordinating agent reported these normalized GPU commands. Their complete
original environments were not retained, so no additional flags are inferred:

```sh
cargo clippy --features gpu --locked --all-targets -- -D warnings
cargo doc --features gpu --locked --no-deps --document-private-items
```

## Package snapshot and retained failures

The final Cargo include filter excludes GPU research artifacts from the published
crate while keeping every report, PNG and failure tracked in the repository.
The final archive passed Cargo verification, all 533 unpacked library tests and
size/path/source-content checks. The SHA-256 and exact content list are in the
[final policy result](final-package-policy.json) and [contents](final-package-contents.txt).
Later edits to these excluded research records do not change that archive.

Earlier snapshots remain available: [full verification](package-verified.log),
[533 unpacked tests](package-unpacked-lib-tests.log) and
[the older policy result](package-verified-policy.json) cover the pre-filter
5,170,011-byte / 660-entry archive. The earlier
[archive-only log](package-check.log) used `--no-verify` for a smaller 305-entry
snapshot. These are historical records, superseded by the final archive checks.

The first default run was rejected because GPU source changed while the format
corpus was rendering. Its [failure log](default-tests-source-change-failure.log)
is retained; the complete frozen-source rerun passed all 941 tests. No pixel
baselines were changed to obtain that pass.

The initial WASI command omitted `-ffreestanding`, causing bundled Zstandard's C
build to look for host libc headers under the WASI target. That
[failure log](wasi-missing-flag-failure.log) is retained. The successful retry used
the flag already set by the repository's web builder; it did not change source
or install a new SDK.

## Reproduce archived-data validation

From this directory, the profile reconstruction writes a new report outside the
archive:

```sh
python3 ../summary.py ../nas-results --results ../nas-results --manifest ../nas-results/manifest-all.json --output /tmp/catwalk-nas-profile-recheck.json
python3 ../compare_results.py ../nas-results --output /tmp/catwalk-nas-image-recheck.json
```

The profile command is expected to return zero. The image command is expected to
return nonzero for the recorded strict qualification failure. Neither command
reruns hardware rendering or changes the frozen policy.
