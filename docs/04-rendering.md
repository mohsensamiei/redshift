# 04 — Rendering and Art Direction

Two requirements drive everything here, and they happen to point the same way:

1. **Preserve the nostalgia.** It must read as the game you remember — same camera, same
   silhouettes, same colour language.
2. **Stay light.** The laptop must stay cool and the fan must stay off.

These are not in tension. The original had flat colours, no real-time shadows, and no
post-processing — precisely the choices that are cheapest to render. Being faithful *is* being
light.

## Why 3D at all

The units are drawn as 3D meshes with a fixed dimetric camera, rather than as sprites. This is
not a step toward a modern look — it is a production decision:

- A sprite-based unit needs every rotation × every animation frame pre-rendered. A mesh gives
  all angles for free.
- Resolution stops mattering. Sharp at any display size, smooth zoom.
- **Adding a new faction becomes a data-and-art task, not a pipeline task** — drop in a model
  file and a rules entry. This is a stated project goal.

The visual style still targets the original's look. 3D is the delivery mechanism, not the
aesthetic.

## Art direction

Locked decisions:

| Aspect | Decision |
|---|---|
| Camera | Fixed dimetric angle matching the original. No free rotation. |
| Zoom | Limited range (roughly 0.75×–1.5×), smooth, with a snap-to-default key. |
| Shading | Flat / cel-style. One directional light. No specular, no PBR. |
| Shadows | Simple soft blob decal under each unit. **No real-time shadow maps.** |
| Post-processing | None. No bloom, no SSAO, no motion blur, no depth of field. |
| Anti-aliasing | MSAA 2× or 4× only — cheap on tiled GPUs, and enough given flat shading. |
| Textures | Minimal. Mostly flat vertex colours with a small palette texture per faction. |
| Poly budget | ~300–1500 triangles per unit, ~2000–5000 per building. |
| Colour | Saturated, high-contrast, readable at a glance. Team colour via a palette swap. |
| UI | Same layout language as the original: right sidebar, minimap, build tabs. |

**Silhouette readability is the single most important art rule.** A unit must be identifiable
from its outline alone at default zoom. This is what the original did well and what most
"remastered" fan projects lose when they add detail. Detail that muddies the silhouette is a
regression, no matter how good it looks in isolation.

## The performance budget

This is a **hard requirement, enforced by an automated check** — not an aspiration. A change
that exceeds it is a bug in the change, not a reason to raise the budget.

| Metric | Ceiling | Notes |
|---|---|---|
| Frame rate | 60, vsync-locked | Never uncapped. See below. |
| Frame time | < 8 ms at 60 Hz | Leaves comfortable headroom |
| Sim tick time | < 5 ms with 400 units | Measured on the reference machine |
| Draw calls | < 500 | Achieved via GPU instancing of same-type units |
| Triangles on screen | < 300,000 | |
| Real-time shadow maps | 0 | Blob decals only |
| Post-processing passes | 0 | |
| Resident memory | < 500 MB | |
| Idle GPU utilisation | < 30% | Reference machine, typical battle |

**Uncapping the frame rate is the single biggest cause of laptop heat in a scene this light.**
An uncapped renderer will happily draw 300 fps, saturating the GPU for no perceptible benefit.
Vsync is on by default and there is no option to disable it.

Additionally:
- The game throttles to 30 fps when the window loses focus.
- On battery power, an optional power-saver mode caps at 30 fps.

### Enforcement

A benchmark scene (a scripted battle with a fixed unit count) runs in CI and on demand:

```sh
cargo run -p redshift-app -- --bench
```

It reports every metric above against its ceiling and exits non-zero on a breach. An on-screen
overlay (`F3`) shows the same numbers live during normal play, so the budget is visible while
developing rather than discovered at the end.

## Where the ceiling actually is

Measured, not assumed, with combat and pathfinding both active:

| Units | Mean tick | Worst tick | Verdict |
|---|---|---|---|
| 400 | 0.35 ms | 2.0 ms | comfortable — 24× realtime headroom |
| 800 | 0.72 ms | 2.8 ms | comfortable |
| 1200 | 1.03 ms | 5.7 ms | **over the 5 ms ceiling** |

The mean scales linearly, so the ceiling is not a throughput wall — it is the
first tick, where every unit asks for a path at once. That is worth knowing
before raising the unit cap: the fix would be spreading the initial path
requests over several ticks, not making pathfinding faster.

Target selection is quadratic in principle — each unit scans the field for an
enemy — but only runs when a unit has no valid target, so it amortises away
once combat settles. A scenario with constant target churn would expose it.

## Instancing

With hundreds of units of a few dozen types, per-unit draw calls would dominate. Units of the
same type and team share a mesh and material and are drawn in a single instanced call with
per-instance transform and team colour. This is what keeps the draw-call count in the hundreds
rather than the thousands, and it is why the poly budget above is generous.

## Interpolation

The sim runs at 20 Hz; the renderer at 60 Hz. The renderer keeps the last two sim states and
interpolates position and rotation between them using a fractional factor.

This is presentation-only and uses floats freely — it never feeds back into the sim. Without it
the game would look like it runs at 20 fps; with it, motion is smooth while the simulation stays
coarse and cheap.

## Two things learned by actually running it

Both were found by building the Phase 0 shell and looking at the result, and
both are the kind of thing that is invisible in code review.

### Tone mapping is off, deliberately

The renderer sets `Tonemapping::None`. Tone mapping exists to compress
high-dynamic-range lighting into display range; this scene is deliberately flat
and low-dynamic-range, so there is nothing to compress and passing colours
through unchanged means the art defines exactly what reaches the screen.

It also has to be off for a practical reason: the default tone curve needs
lookup textures that a trimmed feature set does not ship, and a tone mapping
pass that fails takes the entire 3D pass with it — every surface renders as the
fallback magenta. That failure looks like a material bug, which is a long way
from where the actual problem is.

**Consequence:** light intensities must be calibrated for an untonemapped
pipeline. The usual illuminance figures assume the tone curve is present, and
with it absent they clip every surface to white.

### Frame time is not a measure of load

Vsync is always on, so frame time settles at the display's refresh interval no
matter how little work a frame does. On a 120 Hz panel an idle game reports
8.33 ms — which against a fixed 60 Hz ceiling reads as a permanent breach.

The overlay therefore compares frame time against the *observed* refresh
interval plus a tolerance. What is being checked is that the game keeps up with
the display, not that it hits an arbitrary millisecond count. A dropped frame
doubles the interval and is still caught.

The metric that actually measures our own cost is the simulation tick time,
which is not vsync-bound. That is what `--bench` reports, and why `--bench`
runs headless.

## What is deliberately not here

To keep the budget and the look:

- No normal maps, no PBR materials, no image-based lighting.
- No dynamic lights beyond the single directional one. Explosions flash via emissive colour
  and a decal, not a point light.
- No particle systems with thousands of particles. Effects are a handful of animated quads.
- No terrain tessellation or displacement. Terrain is a simple heightmapped mesh.
- No screen-space effects of any kind.

If a feature request would violate the budget, the answer is no. The budget is the feature.
