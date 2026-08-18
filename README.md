# Redshift

A cross-platform, open-source real-time strategy engine and game in the spirit of the
classic 2000-era isometric RTS: same camera, same pacing, same readable chunky silhouettes —
rendered cleanly at modern resolutions, with LAN and internet multiplayer.

> **Codename.** "Redshift" is a provisional working name. The shipped project must not use
> "Command & Conquer", "Red Alert", or any EA trademark. See [docs/00-overview.md](docs/00-overview.md).

## What this is

- A **faithful remaster**, not a reimagining. Gameplay rules, unit roles, economy and pacing
  stay true to the original. We modernise the engine and the presentation, not the design.
- **Lightweight by mandate.** Flat-shaded low-poly 3D under a hard performance budget.
  Target: a fanless-quiet experience on an Apple M1 Pro. See [docs/04-rendering.md](docs/04-rendering.md).
- **100% original assets.** No game files from any commercial release are required,
  bundled, or redistributed. See [docs/06-assets.md](docs/06-assets.md).
- **Deterministic lockstep multiplayer** over LAN and internet, with replays for free.
  See [docs/03-networking.md](docs/03-networking.md).

## Status

**Phase 0 — Foundation.** Nothing is playable yet. See [TODO.md](TODO.md) for the live task list
and [docs/07-roadmap.md](docs/07-roadmap.md) for the phase plan.

## Quick start

```sh
cargo run -p redshift-app          # the game client
cargo run -p redshift-server       # relay + lobby server
cargo test --workspace             # all tests, including determinism suite
```

Requires a recent stable Rust toolchain (1.95+).

## Documentation

| Document | Contents |
|---|---|
| [docs/00-overview.md](docs/00-overview.md) | Vision, scope, explicit non-goals, legal posture |
| [docs/01-architecture.md](docs/01-architecture.md) | Crate layout and the sim/presentation split |
| [docs/02-simulation.md](docs/02-simulation.md) | Determinism rules, fixed-point math, tick loop |
| [docs/03-networking.md](docs/03-networking.md) | Lockstep, LAN discovery, relay server, desync |
| [docs/04-rendering.md](docs/04-rendering.md) | Art direction and the performance budget |
| [docs/05-data-and-modding.md](docs/05-data-and-modding.md) | Data-driven units, buildings, factions |
| [docs/06-assets.md](docs/06-assets.md) | Art pipeline and asset production |
| [docs/07-roadmap.md](docs/07-roadmap.md) | Phases, milestones, exit criteria |
| [docs/08-roster.md](docs/08-roster.md) | The world the engine has to hold — terrain, civilians, countries, roster, and every gap between them and the engine |
| [docs/adr/](docs/adr/) | Architecture Decision Records — why each choice was made |

Contributors should start with [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

Code: GPLv3 (matching the lineage of the open-sourced Westwood engines).
Original art and audio assets: CC BY-SA 4.0.
