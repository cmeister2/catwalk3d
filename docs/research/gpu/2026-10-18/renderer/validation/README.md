# Validation evidence

These records cover code compatibility, package verification and reproducibility
of archived measurements. They do not qualify production GPU image output: all
12 supported NAS CPU/GPU pairs failed the unchanged strict image gate. See the
[NAS recommendation](../nas-recommendation.md) and
[archived comparisons](../nas-results/comparisons.json).

Source versions differ where recorded: the NAS runs and GPU regression artifacts
identify `6e00d13`; the final GPU Clippy check followed the test-only `ec68f04`
cleanup. Later documentation and Windows evidence additions are outside the
recorded package snapshot. Consult each build/source manifest rather than
assuming every check was run against the final documentation tree.

| Check | Recorded result | Evidence and scope |
| --- | --- | --- |
| Default CPU, all targets | 941 passed; no failures or ignored tests | [Full log](default-tests.log), including the complete format snapshot corpus, integration tests and default examples. |
| WASI library without default features | Passed | [Full log](wasi-check.log); Clang plus the existing web builder's `-ffreestanding` flag. |
| GPU software GL library tests | 16 passed, including 18 image cases | [Metadata and commands](../depth-precision/post-guard-metadata.json), [test log](../depth-precision/post-guard-software-gl.log); software correctness only. |
| GPU-feature CPU profiling parity | 1 passed | [Exact-output parity log](../depth-precision/post-guard-cpu-profile.log). |
| GPU dispatch/shared-scene integrations | 17 passed | [Log](../depth-precision/post-guard-integrations.log): 5 dispatch tests, 7 indexed/instanced tests, 5 shared STEP tests. |
| NAS hardware GL library tests | 16 passed, including 18 image cases | [Output](../nas-correctness-6e00d13/stdout.txt), [exit status](../nas-correctness-6e00d13/exit-status.txt), [run script](../nas-correctness-6e00d13/run.sh). These cases do not replace the failed full CLI image matrix. |
| GPU Clippy, all targets | Passed, warnings denied | Coordinating agent directly observed session `29812` after the test-only cleanup; the complete command environment and raw log were not retained. |
| GPU private-item documentation | Passed, warnings denied | Coordinating agent directly observed session `25299` on `6e00d13`; the complete command environment and raw log were not retained. |
| Full Cargo package verification | Passed | [Log](package-verified.log): Cargo built the unpacked published crate successfully. |
| Unpacked crate library tests | 533 passed; no failures or ignored tests | [Full log](package-unpacked-lib-tests.log); default CPU library tests from the generated package, not the working checkout. |
| Verified package archive policy | Passed | [Policy result](package-verified-policy.json), [contents](package-verified-contents.txt): 5,170,011 bytes, 660 entries, no unexpected or forbidden paths; 10,000,000-byte exclusive limit unchanged. |
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

The verified archive includes the NAS research evidence available at packaging
time. Its SHA-256 is recorded in [the policy result](package-verified-policy.json).
Later Windows evidence and documentation additions require a final archive
refresh and size/path policy check; this snapshot's byte count must not be
presented as verification of those later additions. The earlier
[archive-only package log](package-check.log) used `--no-verify` and covered a
smaller 305-entry archive; it is superseded by the full verification above for
code/package-build coverage.

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
