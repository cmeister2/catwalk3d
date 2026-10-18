# Collapsed placement depth regression

This is a **software OpenGL correctness experiment**, using llvmpipe. It makes no
hardware performance claim. The two opaque unlit panels have different colors and
are separated by `1e-9` along the camera direction. Their original double-precision
placements differ, but all twelve packed single-precision affine projection row
values are identical. CPU rendering shows the closer blue face; GPU depth testing
retains the red face drawn first. The pre-guard test failed with a mean byte error
of 22.7421875 and 3,936 unmatched pixels out of 12,288.

`pre-guard-software-gl.log` preserves the test output, and the two PNGs preserve
the mismatch. `pre-guard-metadata.json` records the command, adapter, exit status,
binary hash and compiled source manifest. Apply `pre-guard-test.patch` to commit
`74724fc` in a separate checkout to reconstruct the failing test source. The patch
adds the measured test only; it does not add the guard.

Commit `6e00d13` adds a conservative **whole-scene CPU fallback**, not an exact GPU
depth fix. CPU preparation interns local coordinate bases and tracks packed
projection rows in a temporary table proportional to occurrence count. Distinct
original source transforms sharing the same basis and packed projection rows are
rejected. Exact source transform ties stay supported, and differing mesh bases
are kept separate. All shader and buffer layouts remain unchanged.

The post-guard test requires rejection of the measured case, then renders controls
for equal source placements, distinct local mesh bases and a resolvable `1e-3`
depth separation. All 16 GPU library tests passed on software GL, including 18 CPU/GPU image
comparisons and the demonstrated fallback. Full logs, unchanged compiled source
manifests, and binary hashes are recorded in the post-guard files. The stripped
artifact also passed the depth regression, while requiring hardware with forced
software correctly failed with exit 101. Arbitrary near-coincident surfaces,
including near-coincident faces within one mesh, remain outside the qualified
scope. This guard does not prove general CPU/GPU depth equivalence.
