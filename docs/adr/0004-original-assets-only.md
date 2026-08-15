# ADR 0004 — Original assets only

**Status:** Accepted · **Date:** 2026-08-15

## Context

Comparable projects take one of two approaches to game content:

1. **Bring your own files.** The engine is open source; the player must own and supply the
   original game's data files. This is what OpenRA, Chrono Divide and the community online
   clients do.
2. **Original assets.** Everything shipped is created by the project.

An important factual correction shaped this decision: **the source code for this era's engine was
never released.** The publisher open-sourced two earlier titles in 2020, but not this one. None
of the reference projects are forks — all are clean-room reimplementations. There is no existing
codebase or asset set to legitimately build on.

The project's stated intent is a redesign at modern quality, and not to require players to own
the original.

## Decision

**The project ships only assets it creates.** No models, textures, sprites, audio, video or data
files from any commercial release are bundled, converted, or committed to this repository.

Gameplay *values* — costs, damage numbers, speeds, build times — are functional game mechanics
rather than creative expression. They are re-derived into our own RON format. The original data
files themselves are not redistributed.

All user-visible naming must avoid trademarks before public release. Because every string goes
through localisation keys ([05-data-and-modding.md](../05-data-and-modding.md)), renaming is a
data change and can be deferred without accruing technical debt. "Redshift" is a provisional
codename.

Licensing: code under GPLv3 (matching the lineage of the publisher's own open-sourced engines);
original assets under CC BY-SA 4.0.

## Consequences

**Positive**

- The project is freely distributable. Anyone can download and play with no other purchase or
  installation — which is what was wanted.
- No dependency on the availability, versioning or file layout of a commercial release.
- Art direction is unconstrained by the original's technical limits, so it can target modern
  displays directly rather than upscaling.

**Negative**

- Art is the dominant cost of the project. A full roster is a large volume of models, textures,
  animations, voice lines and music.
- This is the main reason Phase 4 and Phase 5 carry the widest schedule uncertainty in
  [07-roadmap.md](../07-roadmap.md).

**Mitigations**

- Placeholder primitives through Phases 0–3, so no art is produced before the game beneath it is
  proven.
- Deliberately simple, uniform art style — low-poly, flat-shaded, mostly untextured
  ([ADR 0002](0002-realtime-3d-under-a-budget.md)). Sustainable across a hundred assets by a
  small team, and consistent enough to read as intentional.
- Parametric and scripted modelling in Blender where model families share structure.
- A reduced launch roster is the primary schedule lever if needed.

## Note

This decision is what makes the difference between a project that can be shared publicly and one
that can only be distributed to people who already own the original. Given the effort involved
in everything else, that is worth the additional art cost.
