# 00 — Overview

## Vision

A cross-platform, open-source remaster of a classic 2000-era isometric real-time strategy game.
The original's design is preserved; the engine and presentation are rebuilt from scratch with
modern tooling, modern resolutions, and working multiplayer.

The guiding sentence: **it should feel like memory feels** — the way the game looks in your head
when you remember it, not the way it actually looked on a 800×600 CRT.

## Goals

1. **Faithful gameplay.** Unit roles, economy, build order, pacing, and balance follow the
   original. This is a remaster, not a redesign.
2. **Clean modern presentation.** No pixelation, no aliased sprite edges, smooth zoom, sharp on
   any display — while keeping the original's camera angle, silhouettes, colour language and UI layout.
3. **Genuinely lightweight.** Must run smoothly and *quietly* on an Apple M1 Pro with integrated
   graphics. Enforced by a hard performance budget, not by good intentions.
4. **LAN multiplayer.** Two machines on the same Wi-Fi discover each other and play, with zero
   configuration and no internet connection.
5. **Internet multiplayer.** A lobby and relay server so players on different networks can play
   without port forwarding.
6. **Extensible factions.** Adding a new country is a data and art task, not an engineering task.
7. **Cross-platform.** macOS (Apple Silicon and Intel), Windows, Linux.

## Non-goals

Explicitly out of scope. Listing these protects the schedule.

- **Not a graphical reimagining.** No photorealism, no PBR, no cinematic post-processing.
  If a feature would look at home in a modern AAA title, it does not belong here.
- **Not mobile.** Touch input is a poor fit for this genre. Desktop only.
- **Not a free camera.** Fixed dimetric angle with limited zoom, as the original had. Free
  rotation would break the readability that flat isometric art depends on.
- **Not compatible with original game files.** We do not read or require any commercial
  release's data. See [06-assets.md](06-assets.md).
- **Not rebalanced.** Balance changes are deferred until the faithful baseline is playable
  and proven.
- **No campaign at first.** Skirmish and multiplayer come first; scripted missions are a
  later phase, if at all.

## Scope of "new faction"

The user wants to add new countries. Within the original's framework this means, per country:

- one unique unit or structure,
- one passive advantage (a cost, speed, range, or armour modifier),
- a set of voice lines and a flag/colour identity.

This is deliberately modest and matches how the original handled its countries. A full new
*side* (a third parallel tech tree) is a much larger undertaking and is a Phase 5+ question.

## Legal posture

Being clear about this early prevents wasted work.

- The source code of this era's engine was **never released**. Every comparable project
  (OpenRA, Chrono Divide, and others) is a clean-room reimplementation, not a fork. So is this one.
- We ship **only assets we create**. No models, textures, sprites, audio, video, or data files
  from any commercial release are bundled, converted, or committed to this repository.
- Gameplay *values* — costs, damage, speeds — are functional game mechanics, re-derived and
  stored in our own format. The original data files are not redistributed.
- The project name and all in-game naming must avoid EA trademarks. "Redshift" is a provisional
  codename; unit and faction names will need original equivalents before any public release.
- Code is GPLv3; original assets are CC BY-SA 4.0.

## Target hardware

The reference machine is an **Apple M1 Pro, 16 GB, Metal 3**. The project targets a comfortable
margin on this machine — not "it runs", but "the fan stays off". Anything meaningfully weaker
than this reference is best-effort.

## Related work

Studied for reference, none used as a base:

| Project | Approach | Why not a base |
|---|---|---|
| OpenRA | C# engine, mature lockstep netcode, YAML mods | 2D sprite renderer; resolution ceiling; no path to clean 3D |
| Chrono Divide | Browser reimplementation, TypeScript/WebGL | Closed source; requires original game files |
| Various web ports | Asset-driven browser clients | Require original game files; limited scope |

The valuable lesson from all of them: the hard part is never the rendering. It is deterministic
simulation and netcode. This project's phase order reflects that. See [07-roadmap.md](07-roadmap.md).
