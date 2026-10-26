# Legacy filtered HDF5 fixtures

These small files preserve real version-4 Fixed Array, B-tree v2 and string/CF
regression inputs independently of the currently installed writer. Writer 0.9.3
switched chunked output to version-3 layouts with v1 B-tree indexes. The tests
also build current-writer files and verify their values, slices, strings,
NetCDF masking/unpacking and Alembic rendering.

The fixtures were generated using the independent published writers below;
`generator/Cargo.lock` fixes the full dependency graph. No external data is used.

| Crate | Version | Published crate SHA-256 |
|---|---|---|
| hdf5-writer | 0.9.1 | `a292270e76950c2160cc2ea996cb0a4511299b3cc1481b59e40828a4dc9fcf82` |
| netcdf-writer | 0.9.1 | `23183a2ee350dfaf042f7d3b2b07623ffe20124fd6a70e7fd2255b87a5691d9a` |
| hdf5-core | 0.9.1 | `c91b525024c4692a02df5164c5fc29ee6a01546b6afc1be1d4401186c4099d00` |
| netcdf-core | 0.9.1 | `25a7215ebd56ede9f76768a895593673ca072b67764f3abff7393be0de0dbaf4` |

- `field-fixed.h5`: signed big-endian field, finite maximum extents, filtered fixed array.
- `field-tree.h5`: the same field with two unlimited axes, filtered B-tree v2.
- `field-extensible-source.h5`: one unlimited axis. The test replaces its writer-emitted B-tree v2 index with an independently authored extensible array, preserving the existing index regression.
- `strings.h5`: filtered scalar fixed-width and variable-length UTF-8 strings.
- `density.nc`: packed NetCDF-4 density with fill values, valid range, scale and offset.
- `alembic.h5`: Alembic point properties plus fixed-width and variable-length strings.

`SHA256SUMS` records every fixture. From the repository root, reproduce the files
in an empty directory and verify them with:

```sh
cargo run --locked --manifest-path tests/fixtures/hdf5-owned/generator/Cargo.toml -- /tmp/catwalk-hdf5-fixtures /tmp/catwalk-hdf5_legacy_data.rs
rustfmt --edition 2024 /tmp/catwalk-hdf5_legacy_data.rs
cmp src/formats/hdf5_legacy_data.rs /tmp/catwalk-hdf5_legacy_data.rs
(cd /tmp/catwalk-hdf5-fixtures && sha256sum -c "$OLDPWD/tests/fixtures/hdf5-owned/SHA256SUMS")
```

The generator is a separate workspace, so the main package continues to use the
current 0.9.3 dependencies. Its optional second argument emits base64 constants
for `src/formats/hdf5_legacy_data.rs`. Library tests decode those source constants,
so source archives remain self-contained while the existing package allowlist
continues to exclude fixture and test directories. The binaries here remain the
small, independently checksummed provenance for the generated constants.
