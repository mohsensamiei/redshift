# CLAUDE.md

Guidance for AI assistants (and humans) working in this repository.
Read this before touching code. The rules in **Hard invariants** are not stylistic preferences —
breaking any of them silently corrupts multiplayer or the project's legal footing.

## What this project is

A cross-platform remaster of a classic 2000-era isometric RTS. See [README.md](README.md)
and [docs/00-overview.md](docs/00-overview.md). Two things define every technical decision:

1. **Faithful, not reimagined.** Gameplay rules match the original. When in doubt about a
   design question, the answer is "what did the original do?" — not "what would be better?"
2. **Light by mandate.** The user's stated requirement is that the laptop stays cool and quiet.
   Every rendering feature is opt-out by default. See the budget in [docs/04-rendering.md](docs/04-rendering.md).

## Hard invariants

### 1. No floating point in the simulation — ever

The multiplayer model is deterministic lockstep. Every client must produce a **bit-identical**
simulation from the same inputs, across x86 and ARM. A single `f32` in a sim code path will
desync matches minutes later, in a way that is extremely expensive to debug.

- `redshift-sim` must never depend on `f32`/`f64`. Use `Fx` (see [docs/02-simulation.md](docs/02-simulation.md)).
- A CI lint denies float types in that crate. Do not silence it.
- Floats are fine in `redshift-render` — presentation only.

### 2. The simulation never depends on the engine

`redshift-sim` must not depend on Bevy, wgpu, winit, or any rendering/windowing crate.
It must compile and run headless. This gives us the dedicated server, replays, automated
tests, and immunity to Bevy's API churn. Check with:

```sh
cargo tree -p redshift-sim | grep -i bevy    # must return nothing
```

### 3. Determinism hygiene

Inside the simulation:

- **No `HashMap`/`HashSet` iteration.** Their order is randomised per process. Use `BTreeMap`,
  `Vec`, or a slotmap with stable indices.
- **No wall-clock time.** No `Instant::now()`, `SystemTime`, or elapsed-time-driven logic.
  The only clock is the tick counter.
- **No thread-order dependence.** Parallelism is allowed only where results are order-independent
  and reduced deterministically.
- **One RNG.** A single seeded PCG generator lives in sim state and is advanced only by sim code.
  Never `rand::thread_rng()`.
- **No `sort_unstable_by` on partial orders.** Sorts must have a total, deterministic tie-break
  (fall back to entity id).

### 4. Ship the faithful remaster before changing anything

The country roster is **exactly** the original's: America, Korea, France,
Germany, Great Britain, Russia, Iraq, Libya, Cuba. No additions, no removals.

New countries, new units and rebalancing are all wanted, and all come **after**
the remaster ships. A remaster that is "nearly faithful plus some new things"
cannot be checked against anything, because there is no longer a version of the
game to compare it to.

If a request arrives for new content mid-remaster, the answer is to record it
for the content phase rather than to build it. See
[docs/adr/0005-faithful-remaster-scope.md](docs/adr/0005-faithful-remaster-scope.md).

### 5. No third-party game assets

This project ships only assets we created. Do not add, extract, convert, or commit art, audio,
video, or data files originating from any commercial release. Gameplay *values* (damage numbers,
build costs, speeds) are re-derived and stored in our own data format; the original data files
themselves are never redistributed.

### 6. The performance budget is a test, not a wish

See [docs/04-rendering.md](docs/04-rendering.md). The budget is enforced by an automated check.
If a change blows the budget, the change is wrong — not the budget.

## Repository layout

```
crates/
  sim/       redshift-sim     deterministic game simulation. Zero engine deps. The heart.
  data/      redshift-data    rules: units, buildings, factions. RON files + loaders.
  net/       redshift-net     lockstep transport, LAN discovery, relay client, replays.
  render/    redshift-render  Bevy plugin. Reads sim state, draws it. Never mutates sim.
  app/       redshift-app     the client binary. Wires everything together.
server/      redshift-server  headless relay + lobby. Depends on net, not on render.
assets/                       original models, textures, audio (see docs/06-assets.md)
docs/                         architecture and design docs; docs/adr/ for decisions
```

Data flows **one way**: `sim` → `render`. The renderer holds an immutable view of sim state and
an interpolation factor. It never writes back. Player input becomes a *command*, which enters
the sim only through the network layer's ordered command queue — even in single-player.

## Commands

```sh
cargo run -p redshift-app                 # run the client
cargo run -p redshift-server              # run relay + lobby
cargo test --workspace                    # full test suite
cargo test -p redshift-sim determinism    # determinism suite (run before any sim change lands)
cargo clippy --workspace -- -D warnings    # lint
cargo fmt --all                           # format
```

## Conventions

- Rust 2024 edition. `rustfmt` defaults. Clippy clean at `-D warnings`.
- Documentation is written in English so the repo stays contributable; conversation may be
  in any language.
- Every non-obvious decision gets an ADR in [docs/adr/](docs/adr/). Prefer amending an existing
  ADR over contradicting it silently.
- Public items in `redshift-sim` and `redshift-data` carry doc comments. Elsewhere, comment
  *why*, not *what*.
- Commit messages: imperative mood, scoped — `sim: add fixed-point sqrt`.

## Working style for AI assistants

- **Small, verifiable steps.** Prefer one crate at a time with tests, over broad scaffolding.
- **Never weaken a determinism test to make it pass.** A failing determinism test is a real bug.
- **Do not add dependencies casually.** Each one is a maintenance and binary-size cost. New
  deps in `redshift-sim` need a very good reason and must be `no_std`-friendly where possible.
- **Update [TODO.md](TODO.md)** when you complete or discover work.
- When the original game's behaviour is unclear, flag it rather than inventing a design.
