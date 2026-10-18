# Renderer build identities

[linux.json](linux.json) records the completed release build at source checkpoint
`6e00d1371992fff6788c0e914de040192d3986c8`, compiler output, command, source-file
hashes, unchanged inputs across compilation, and executable hashes. Those exact
Linux executables produced the archived NAS renderer matrix. The image-test
harness has its own identity under
[nas-correctness-6e00d13](../nas-correctness-6e00d13/metadata.json).

Checkpoint `ec68f04` changes only a test assertion's Rust syntax from that
production source. The completed native MSVC release build is recorded in
[windows.json](windows.json) and [windows.log](windows.log): Rust 1.99.0, 127
captured inputs unchanged during the build, and separate executable hashes.
The native Windows Cargo cache
was moved from a WSL UNC target directory to a unique NTFS scratch directory after
Cargo's generated build-script executable copy failed with access denied. No
production source change was needed for that cache migration.
The [failed copy log](windows-unc-copy-failure.log),
[cache migration](windows-cache-migration.json) and
[native timestamp repair](windows-cache-timestamp-repair.json) preserve that
environmental failure and its resolution; they are not GPU compatibility tests.

[record-build.py](record-build.py) is the build recorder used for these runs. Its
absolute paths describe this investigation's environment; edit them for another
machine. It hashes source before and after building and emits completed metadata
only after both executables exist and source hashes agree. An evolving worktree,
successful capability probe, or old executable does not substitute for a
completed renderer build.

Subsequent README qualification wording and the Cargo package include filter do
not describe a new renderer executable. GPU research artifacts remain tracked in
the repository but are excluded from the published crate to preserve its existing
10,000,000-byte size limit. Runtime source and dependency versions are unchanged
by that packaging adjustment; use the recorded manifests for the actual builds.
