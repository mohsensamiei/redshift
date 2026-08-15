# ADR 0003 — Deterministic lockstep networking

**Status:** Accepted · **Date:** 2026-08-15

## Context

The project requires both LAN multiplayer (two machines on the same Wi-Fi, zero configuration)
and internet multiplayer. We need a network model.

The two viable models for a real-time strategy game:

| Model | Bandwidth | Server cost | Determinism required | Fits RTS |
|---|---|---|---|---|
| **Deterministic lockstep** — send commands only | Tiny, constant regardless of army size | None (or a dumb relay) | **Yes, absolutely** | Yes — the genre standard |
| **Authoritative server state replication** | Scales with entity count | High — server simulates every match | No | Poorly at RTS scale |

State replication is what shooters use, and it works because they have tens of entities. An RTS
has hundreds to thousands. Replicating every unit's position for every player, several times a
second, is both expensive to host and expensive in bandwidth — and it would require us to run and
pay for servers that actually simulate the game.

## Decision

**Deterministic lockstep**, with command scheduling at a negotiated input delay, over UDP.
Internet play uses a **relay server that is a packet switch, not a game authority**.

Key properties:

- Peers exchange only player commands. A 200-unit move order is one command, not 200 updates.
- Bandwidth is a few hundred bytes per second per player, identical on LAN and internet.
- Commands issued at tick `N` execute at tick `N + D`; `D` is negotiated from measured RTT and
  fixed for the match.
- The renderer acknowledges input instantly (selection sound, move marker) even though the unit
  acts `D` ticks later. This is why input delay is nearly invisible in this genre.
- Single-player runs the identical path with one peer and zero delay, so the multiplayer code
  is exercised continuously rather than integrated at the end.

The relay deliberately holds no game state. It cannot desync, it is cheap to host, and a bug in
it cannot corrupt a match.

## Consequences

This is the decision that constrains the entire codebase.

- **Floating point is banned in the simulation.** Float results are not reliably reproducible
  across architectures, and we target both ARM and x86. Everything uses fixed-point `Fx`.
  Enforced by a CI lint.
- **No hash-map iteration, no wall-clock time, no thread-order dependence, one seeded RNG** in
  sim code. See [02-simulation.md](../02-simulation.md).
- **The simulation must be engine-independent and headless-capable.** This is what makes
  [ADR 0001](0001-rust-and-bevy.md)'s sim/presentation split mandatory rather than merely tidy.
- **Pathfinding is budgeted in node expansions, never in milliseconds.** A time-based cutoff is
  the most common source of desyncs in RTS codebases.
- **Replays come free** — a match is its seed plus its command log, a few kilobytes. This becomes
  the primary debugging tool for the whole project.
- **A desync halts the match.** There is no partial recovery; the models diverge and cannot be
  reconciled. Detection is therefore built in from Phase 1: hashes exchanged every second, with
  dumps from both peers on mismatch.
- **Cheating by fog-of-war removal is inherent** to the model, since each client holds full world
  state. This was equally true of the original. Full anti-cheat is out of scope.

## Alternatives rejected

- **Rollback netcode** (predict, then re-simulate on correction) suits fighting games with few
  entities. Re-simulating hundreds of RTS units several frames back every time a packet arrives
  is far too expensive, and the genre does not need frame-level responsiveness.
- **Authoritative server**: rejected on hosting cost and bandwidth, as above.

## Validation

Phase 1 does not close until CI runs the same scripted match on macOS/ARM and Linux/x86 and gets
identical state hashes at fixed checkpoints. Until that passes, the project's core premise is
unproven — which is why Phase 1 precedes all gameplay and art work.
