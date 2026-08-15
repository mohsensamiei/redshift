# 06 — Assets

## Policy

Every asset in this repository is created by the project. No models, textures, sprites, audio,
video, or data files from any commercial release are bundled, converted, or committed.

This is both a legal requirement and a practical one — it is what allows the project to be
distributed freely, and it means a player does not need to own or install anything else to play.

Gameplay *values* (costs, damage numbers, speeds, build times) are functional mechanics rather
than creative expression, and are re-derived into our own data format. The original data files
themselves are never redistributed.

Naming also has to be original before any public release: unit, structure and faction names must
not reuse trademarked names. Because everything user-visible goes through localisation keys
(see [05-data-and-modding.md](05-data-and-modding.md)), renaming is a data change, and can be
deferred without creating technical debt.

## Pipeline

```
Blender  ──▶  .glb (glTF 2.0)  ──▶  assets/  ──▶  runtime load
  │              │
  │              └─ flat-shaded, vertex-coloured, team-colour material slot
  └─ modelling, rigging, animation
```

- **glTF 2.0 (`.glb`)** is the interchange format. It is open, well-supported by Bevy, and keeps
  meshes, materials and animations in one file.
- **Blender** is the authoring tool. Free, cross-platform, scriptable — which matters, because
  much of the model production will be parametric rather than hand-sculpted.
- No custom binary asset format until profiling proves load time is a problem.

## Budgets per asset

Derived from the rendering budget in [04-rendering.md](04-rendering.md):

| Asset | Triangles | Textures |
|---|---|---|
| Infantry | 300–600 | none (vertex colour) |
| Vehicle | 600–1500 | optional 256² palette |
| Aircraft | 500–1200 | none |
| Building (small) | 800–2000 | optional 512² |
| Building (large) | 2000–5000 | optional 512² |
| Terrain tile | 50–200 | shared atlas |

Most units should need **no texture at all** — flat vertex colours plus a team-colour slot is
both the cheapest option and the most faithful to the original's look.

## Team colour

One material slot per model is designated the team-colour slot and is tinted per player at
runtime via an instance attribute. This is why units of the same type across different teams
still batch into a single instanced draw call.

## Modelling guidance

- **Silhouette first.** Block out the outline, check it at default zoom as a solid black shape,
  and only then add detail. If it is not identifiable in silhouette, the model has failed
  regardless of how it looks up close.
- **Model for one camera angle.** The camera is fixed and dimetric. Detail on undersides and
  rear faces is wasted budget.
- **Consistency beats quality.** A roster of uniformly simple models looks intentional and
  professional. A roster mixing simple and detailed models looks unfinished. Set the style bar
  at what can be sustained for a hundred assets, not at what one showcase model can reach.
- **Chunky over fine.** Fine detail disappears at gameplay zoom and only costs triangles.

## Animation

Kept minimal and cheap, matching the original's feel:

- Vehicles: turret rotation and wheel/track scroll. No suspension or body sway.
- Infantry: walk, fire, die — short loops, low bone counts.
- Buildings: idle loops (rotating dishes, blinking lights) and a build-up animation.
- Effects: animated quads and decals, not particle systems.

## Placeholders

Phases 0–3 use coloured primitives — boxes, cylinders, capsules — sized to final unit
proportions. This is deliberate:

- It proves the engine, netcode and gameplay before any art investment.
- It keeps early phases fast.
- Correct proportions mean final models drop in without rebalancing selection boxes, collision
  or pathfinding footprints.

Art production starts in Phase 4, once the game underneath is proven. See
[07-roadmap.md](07-roadmap.md).

## Audio

Same policy: original recordings and synthesis only.

- **SFX:** weapon fire, explosions, movement, UI. Synthesised or recorded, then processed to sit
  in the original's punchy, mid-forward register.
- **Voice:** unit acknowledgements and an announcer. Recorded or synthesised; localisation-keyed
  like text, so alternative language sets can be added.
- **Music:** original compositions. Deferred to Phase 4+; the game is fully playable without it.

## Storage

Binary assets are tracked with **Git LFS** to keep the repository clone fast. Source files
(`.blend`) live alongside exports so models remain editable, but only exports are loaded at
runtime.
