# ADR 0002 — Real-time 3D under a hard performance budget

**Status:** Accepted · **Date:** 2026-08-15

## Context

Two requirements were stated and initially appeared to be in tension:

1. The game must not look pixelated — it should be sharp on a modern display.
2. The game must stay light. The laptop must not heat up, and it must not become "a strange
   modern AAA game". The nostalgic design must be preserved.

The rendering options:

| Option | Sharpness | Runtime cost | Cost of a new faction |
|---|---|---|---|
| Upscaled 2D sprites | Limited; upscaling artefacts | Very low | High — every rotation × animation frame |
| Pre-rendered high-res sprites from 3D models | Good | Lowest | High — re-render and re-atlas per unit |
| **Real-time 3D, flat-shaded** | Unlimited | Low | **Low — drop in a model file** |
| Real-time 3D, modern PBR | Unlimited | High | Low |

## Decision

**Real-time 3D with flat/cel shading, a fixed dimetric camera, no real-time shadow maps and no
post-processing — governed by a hard, automatically enforced performance budget.**

The reasoning that resolved the apparent tension:

**3D is not what heats a laptop.** Heat comes from specific features: high-resolution real-time
shadow maps, PBR with many lights, post-processing stacks, and — most of all — an uncapped frame
rate. A flat-shaded low-poly scene at vsync-locked 60 fps barely wakes an M1 Pro's GPU.

**The original's look is already the cheap look.** Flat saturated colours, no real-time shadows,
no post-processing, one light direction. Being faithful and being light are the same set of
choices, not a trade-off.

Real-time 3D was chosen over pre-rendered sprites — despite sprites being marginally cheaper at
runtime — because of the third column: **adding a new country is an explicit project goal.** With
sprites, each new unit means rendering dozens of rotation × animation frames and managing atlases.
With meshes it is one file in a folder. At this level of scene simplicity the runtime difference
is negligible; the development-cost difference is not.

## Consequences

- The budget in [04-rendering.md](../04-rendering.md) is a **test**, not a guideline. A change
  that breaches it is rejected. Notably: vsync is always on, with no option to disable it.
- Art direction is locked to flat shading, limited zoom, fixed camera, blob shadows. Feature
  requests inconsistent with this are declined by default.
- Silhouette readability becomes the primary art review criterion — the failure mode of 3D
  remasters is adding detail that destroys the shape recognition the original depended on.
- Units of the same type and team are drawn with GPU instancing, which is what keeps draw calls
  in the hundreds.
- We forgo normal maps, PBR, dynamic lights, and particle systems. Accepted deliberately.

## Notes

The choice of 3D is a *production* decision — a delivery mechanism that makes resolution
independence and cheap new factions possible. It is explicitly **not** a move toward a modern
graphical style. If the result starts looking like a contemporary AAA title, that is a defect
against [00-overview.md](../00-overview.md), not a success.
