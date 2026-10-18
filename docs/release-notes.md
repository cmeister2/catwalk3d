# Release notes

See [GitHub Releases](https://github.com/cmeister2/catwalk3d/releases) for the complete published version history and downloads.

## Upgrading to 2.x

When migrating Rust code to 2.x, include a wildcard arm in matches on the now non-exhaustive `scene::TextureWrap`. Its new `ClampToBorderWhite` mode preserves the underlying material outside a texture's unit square.

OBJ material `Kd` now uses linear diffuse reflectivity consistently, so older OBJ material thumbnails can become brighter. OBJ previews also retain imported normals, per-corner UVs and RGB, local diffuse image maps and dissolve opacity; see the [OBJ coverage entry](format-coverage.md#obj) for supported options and limits.
