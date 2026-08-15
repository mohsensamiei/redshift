# 01 — Architecture

## The one decision everything else follows from

**The simulation is a pure, deterministic, engine-independent library. Everything else is a
client of it.**

```
                    ┌─────────────────────────────────────┐
                    │           redshift-sim              │
   commands ───────▶│   deterministic, fixed-point        │
   (ordered,        │   headless, zero engine deps        │
    from net)       │   tick(commands) -> ()              │
                    └─────────────────────────────────────┘
                                   │  immutable view
                                   ▼
        ┌────────────────────┬──────────────────┬────────────────────┐
        │  redshift-render   │ redshift-server  │  replay / tests    │
        │  (Bevy, on screen) │ (headless relay) │  (headless, CI)    │
        └────────────────────┴──────────────────┴────────────────────┘
```

Data flows one way. The renderer reads sim state and an interpolation factor; it never writes
back. Player input never touches the sim directly — it becomes a *command* that enters through
the network layer's ordered queue, **even in single-player**. Single-player is simply a match
with one peer and zero input delay. This means the multiplayer path is exercised constantly
during development instead of being bolted on at the end.

### What this buys us

| Benefit | How it falls out |
|---|---|
| Multiplayer | Lockstep requires exactly this separation |
| Dedicated server | Sim runs headless with no renderer |
| Replays | A replay is the seed plus the command log — a few kilobytes |
| Automated testing | Sim is testable without a window or GPU |
| Desync debugging | Two sims can be run in one process and diffed |
| Engine independence | Bevy churns between releases; the sim never notices |

## Crate layout

```
crates/
  sim/       redshift-sim
  data/      redshift-data
  net/       redshift-net
  render/    redshift-render
  app/       redshift-app
server/      redshift-server
```

### `redshift-sim`

The game. World state, units, buildings, movement, pathfinding, combat, economy, fog of war,
victory conditions.

- **Depends on:** `redshift-data`, `serde`. Nothing else of consequence.
- **Forbidden:** any float type, any engine crate, any I/O, any clock, any `HashMap` iteration.
- **Public API surface** is deliberately small:

```rust
pub struct Sim { /* ... */ }

impl Sim {
    pub fn new(setup: &MatchSetup, rules: &Rules) -> Self;
    pub fn tick(&mut self, commands: &[Command]);
    pub fn tick_number(&self) -> u32;
    pub fn state_hash(&self) -> u64;
    pub fn view(&self) -> WorldView<'_>;   // read-only, for renderers
}
```

That is the entire contract. If the renderer needs something, it is added to `WorldView`,
never by exposing mutable internals.

### `redshift-data`

Rules as data. Unit, building, weapon, armour and faction definitions live in RON files and are
parsed into typed structs here. Both the sim and the tooling depend on this. Adding a unit or a
faction should touch this crate's *data files* and no Rust code at all.

See [05-data-and-modding.md](05-data-and-modding.md).

### `redshift-net`

Everything about getting commands between peers in a deterministic order:

- lockstep turn scheduling and input delay,
- LAN discovery over UDP broadcast,
- relay client for internet play,
- state-hash exchange and desync detection,
- replay recording and playback (a replay is just a recorded command stream).

Depends on `redshift-sim` only for the `Command` and hash types.

### `redshift-render`

A Bevy plugin. Owns the window, camera, meshes, materials, UI, audio and input. Translates raw
input into `Command`s and hands them to `redshift-net`. Translates `WorldView` into things on
screen, interpolating between the last two sim ticks.

This is the only crate allowed to use floats freely, and the only one that knows Bevy exists.

### `redshift-app`

Thin binary. Parses arguments, loads rules and assets, constructs the sim, the net session and
the Bevy app, and runs. Should stay under a few hundred lines.

### `redshift-server`

Headless binary: lobby (who is hosting what) and relay (forward command packets between peers
that cannot reach each other directly). Deliberately knows nothing about game rules — it is a
switch, not an authority. This keeps it cheap to host and impossible to desync.

## Threading model

- **Sim:** single-threaded by default. Determinism first; optimise only when profiling proves a
  need, and only with order-independent reductions.
- **Render:** Bevy's normal parallel scheduling. Free to use all cores.
- **Net:** its own thread with a channel to the main loop. Never blocks the sim.

## Time model

Two clocks, deliberately decoupled:

| Clock | Rate | Purpose |
|---|---|---|
| Sim tick | 20 Hz (50 ms), fixed | Game logic. Never varies, never skipped, never subdivided. |
| Render frame | 60 Hz (vsync) | Drawing. Interpolates between the last two sim states. |

A sim tick is an indivisible unit. If a frame takes too long, we render fewer frames — we never
run a partial tick. If the network stalls, the sim waits; it does not extrapolate.

See [02-simulation.md](02-simulation.md) for the tick loop, and [03-networking.md](03-networking.md)
for how input delay hides latency.

## Directory conventions

- `assets/` — original art and audio, organised by kind, tracked with Git LFS.
- `rules/` — RON data files, loaded by `redshift-data`.
- `maps/` — map files (own format, see Phase 3).
- `docs/adr/` — one file per significant decision, never deleted, superseded rather than removed.
