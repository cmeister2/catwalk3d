# NAS GPU pixel-policy failure diagnosis

The frozen GPU image policy remains **failed for all 12 GPU comparisons**. GPU
selection is not production qualified under this policy. The analyzed failures
are rare subpixel triangle-coverage changes at internal facet or occlusion edges;
the concrete worst cases do not indicate a lighting-formula error or a close
depth tie. No source, policy, original PNG or recorded result was modified.

The original results are
`/tmp/catwalk-renderer-bundle-20261018-final/results/comparisons.json`.
The policy SHA256 remains
`88f12e77f2ac5b2d30ad7a4383f309aa47ecd6d67ad388eb2291f3a70f79f38f`.
The recorded renderer source is commit `6e00d1371992fff6788c0e914de040192d3986c8`.
Coordinates below are zero-based output pixels, with the image origin at top left.
Opaque and transparent pairs have the same worst interior coordinates and colors.

| Input | Samples | Interior pixels exceeding max error 3 | Worst coordinate | CPU RGB | GPU RGB | Max error |
|---|---:|---:|---|---|---|---:|
| gear | 1 | 1 | (406,313) | 52,83,115 | 88,141,194 | 79 |
| gear | 2 | 3 | (462,229) | 35,56,77 | 48,77,106 | 29 |
| AP242 | 1 | 1 | (372,301) | 82,133,184 | 60,98,137 | 47 |
| AP242 | 2 | 1 | (154,187) | 94,151,208 | 87,140,193 | 15 |
| repeated ellipsoids | 1 | 20 | (90,302) | 38,61,84 | 45,71,98 | 14 |
| repeated ellipsoids | 2 | 5 | (122,317) | 96,154,211 | 80,128,175 | 36 |

Every pair fails only `interior_max_error`; its other frozen gates pass. The
common-interior mask erodes object/background coverage by one pixel and retains
internal facet and occlusion edges. A one-pixel face-selection change therefore
remains an interior failure even when foreground coverage agrees.

## Concrete geometry evidence

`attribute_geometry.py` independently reconstructs the CPU camera, normalization,
projection, flat legacy shading and f64 barycentric coverage for selected rays.
The JSON records candidate faces, colors, depths, weights and nearest edge
distances. It does not instrument the GPU rasterizer. OBJ polygons are fan
triangulated for this analysis; their planar face boundary and flat colors are
preserved, but importer diagonal ordering is not reproduced.

At gear pixel **(406,313)**, the CPU chooses a wall face colored `[52,83,115]`.
The GPU color `[88,141,194]` is exactly the adjacent top face color, and matches
the surrounding top pixels. Independent projection places the sample center
**0.00086277 output pixels outside that top face**; its minimum CPU barycentric
weight is `-2.21595e-5`. The wall is covered, with positive minimum weight
`1.40780e-5`. The extrapolated top depth is nearer by approximately `4.24e-6` in
normalized CPU depth, so admitting the top face gives precisely the observed
color. This is coverage at a shared projected boundary, not broad shading drift.

At repeated-ellipsoid pixel **(90,302)**, the selected CPU facet color is
`[38,61,84]`. The adjacent facet's flat color is exactly GPU `[45,71,98]`.
The shared boundary is only **0.00019732 output pixels** from the sample center;
the GPU-colored facet has negative CPU minimum barycentric weight `-0.00041331`.
The symmetric pixel `(421,302)` shows the same boundary distance and corresponding
neighboring facet-color substitution.

At repeated-ellipsoid pixel **(122,317), samples=2**, the independently reconstructed
four CPU sample colors are:

```
[100,160,220]  [94,150,206]
[ 94,151,207]  [96,153,210]
```

Their encoded RGB box average rounds exactly to CPU `[96,154,211]`. Replacing
only the first sample with the just-outside foreground instance's flat color
`[35,56,77]` gives **exactly GPU `[80,128,175]`**. That foreground boundary misses
the sample by **0.00008664 output pixels / 0.00017329 supersample pixels**.
The foreground candidate depth is approximately `0.08065`; the CPU-selected
background instance depth is `-0.38307`, a separation of about `0.464`. A close
depth tie cannot explain this substitution. The differing front-face coverage
reveals or occludes a whole high-contrast sample inside the total object mask.

The independent AP242 inspection in `ap242-interior-analysis.json` finds that
the sample-1 worst pixel changes between two exact neighboring face colors in
a 7x7 neighborhood, with no third color. Every other common-interior pixel is
within one byte. Sample 2 likewise has one larger internal-edge error; its other
80 changed interior pixels are within one byte. Matching opaque/transparent
failures exclude background-alpha resolution as the cause.

## Source explanation and limits

CPU legacy coverage uses f64 projected coordinates and edge weights with the
`>= -1e-10` inclusion rule (`src/render.rs`, `Canvas::triangle`). GPU vertices
and projection rows are f32 (`src/gpu/prepare.rs`), and triangle coverage is
performed by the native rasterizer after `mesh_vertex` (`src/gpu/shader.wgsl`).
The nearby, differently colored candidates are outside CPU coverage and become
visible when the hardware coverage decision moves across the boundary. This is
consistent with finite vertex precision and hardware subpixel raster rules.
The exact hardware subpixel quantization has not been independently measured;
these artifacts do not claim a particular fractional-pixel bit count.

The small image tests do not establish the stricter 512px policy. Their helper
accepts per-channel errors up to 4, matches colors anywhere in a surrounding
3x3 neighborhood, permits a fraction of unmatched pixels, and checks a global
mean tolerance. That explicitly tolerates internal edge ownership changes.
Their 128x96 / 192x160 sample alignment also differs from the 512px experiment.
The frozen comparator instead rejects *any* common-interior channel error above
3, without removing internal edges. Passing the unit suite and failing this
policy are therefore consistent observations.

No one-line change to lighting, depth comparison or rounding is supported by
this evidence. Shifting vertices to fix a particular sample can create failures
at other boundaries. More supersampling changes the affected rays and may reduce
average error, but does not guarantee this maximum-error gate (the repeated
scene's observed maximum increases from 14 to 36 at samples 2).

Future substantial options are custom coverage/rasterization implementing the
CPU edge and depth contract, or a conservative CPU correction/fallback path for
ambiguous projected edges. Custom fragment coverage would first need fragments
for samples omitted by native rasterization; a compute/tiled approach or expanded
coverage is more involved than a shader arithmetic adjustment. Portable exact
f64 CPU equivalence and visibility at overlapping edges also need separate work.
A whole-scene CPU fallback is safe for correctness but can erase the measured
GPU benefit. These options require their own design and frozen-policy rerun;
none was implemented or used to change the recorded failures.

## Files and reproduction

Run `python3 analyze_pixels.py` and `python3 attribute_geometry.py` from this
directory while the original bundle and worktree remain at their recorded paths.
Only analysis files under `/tmp/catwalk-pixel-investigation` are written.

`pixel-summary.json` records exact policy violations and original result hash.
`changed-pixels.json` records all differing pixels and their 3x3 neighborhoods.
`gear-geometry-attribution.json` and
`synthetic-repeated-geometry-attribution.json` contain the independent ray evidence.
`*-crop-cpu-gpu-diff.png` shows a 19x19 region enlarged with nearest-neighbor
display: **CPU | GPU | absolute RGB difference multiplied by 8**. These are
diagnostic crops only; the actual comparisons retain the original coordinates.
