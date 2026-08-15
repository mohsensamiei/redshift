# Contributing

Thanks for your interest. This document covers the workflow, the standards, and — most
importantly — the few rules that cannot be bent without breaking multiplayer.

Start by reading [CLAUDE.md](CLAUDE.md) and [docs/01-architecture.md](docs/01-architecture.md).
If you plan to touch `redshift-sim`, [docs/02-simulation.md](docs/02-simulation.md) is required
reading, not optional.

## Setup

```sh
rustup update stable                  # 1.95 or newer
git lfs install                       # binary assets are tracked with LFS
cargo build --workspace
cargo test --workspace
```

Nothing else is needed. There is no external engine or SDK to install.

## Everyday commands

```sh
cargo run -p redshift-app                  # run the client
cargo run -p redshift-app -- --bench       # performance budget check
cargo run -p redshift-server               # relay + lobby
cargo test --workspace                     # everything
cargo test -p redshift-sim determinism     # the determinism suite
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Press `F3` in game for the live performance overlay.

## The rules that cannot be bent

These protect multiplayer and the project's legal footing. A pull request that breaks one will
not be merged regardless of how good the rest of it is.

### 1. No floating point in `redshift-sim`

Multiplayer is deterministic lockstep: every machine simulates the whole game and must reach a
bit-identical result, on both ARM and x86. Floats are not reliably reproducible across
architectures and optimisation levels. A single `f32` in a sim path will desync matches, and it
will do so minutes after the actual divergence.

Use the `Fx` fixed-point type. A CI lint enforces this; do not silence it.

Floats are entirely fine in `redshift-render` — that is presentation.

### 2. No engine dependency in `redshift-sim`

```sh
cargo tree -p redshift-sim | grep -i bevy   # must print nothing
```

The simulation must build and run headless. This is what gives us the dedicated server, replays,
CI testing, and independence from Bevy's API churn.

### 3. Determinism hygiene in sim code

- No iteration over `HashMap`/`HashSet` — their order is randomised per process. Use `BTreeMap`,
  `Vec`, or the entity arena.
- No `Instant::now()`, `SystemTime`, or any wall-clock time. The tick counter is the only clock.
- No thread pools or `rayon`. The sim is single-threaded.
- One RNG, seeded, living in sim state. Never `rand::thread_rng()`.
- Sorts need a total order with a deterministic tie-break — fall back to entity id.
- Budget expensive work (pathfinding) in **node counts, never milliseconds**. Time-based cutoffs
  are the most common cause of desyncs.

### 4. Never weaken a determinism test

A failing determinism test is always a real bug. Fix the cause, not the test.

### 5. No third-party game assets

Only assets created for this project. Do not add, extract, convert, or commit models, textures,
sprites, audio, video or data files originating from any commercial release. See
[docs/adr/0004-original-assets-only.md](docs/adr/0004-original-assets-only.md).

### 6. The performance budget is a test

See [docs/04-rendering.md](docs/04-rendering.md). `--bench` exits non-zero on a breach. If a
change blows the budget, the change is wrong.

## Scope discipline

This is a **faithful remaster**. Gameplay design is fixed — see
[docs/adr/0005-faithful-remaster-scope.md](docs/adr/0005-faithful-remaster-scope.md).

When a design question arises, the answer is "what did the original do?", not "what would be
better?". Balance changes, new mechanics, and quality-of-life features that alter outcomes are
out of scope until the faithful baseline is proven. Please open a discussion rather than a pull
request for anything in that category.

Adding units, buildings or factions should require **no Rust changes** — only RON data and assets.
If you find yourself writing Rust to add content, that is a sign the trait system needs extending;
say so in the issue.

## Workflow

1. Open an issue first for anything non-trivial. It is cheaper to align on approach than to
   review a large PR built on the wrong assumption.
2. Branch from `main`: `feat/short-description`, `fix/…`, `docs/…`.
3. Keep pull requests small and single-purpose. A PR that touches the sim, the renderer and the
   netcode at once is very hard to review for determinism safety.
4. Ensure `cargo test --workspace`, `cargo clippy -- -D warnings` and `cargo fmt --check` pass.
5. Update [TODO.md](TODO.md) if you completed or discovered work.

### Commit messages

Imperative mood, scoped by crate:

```
sim: add fixed-point integer sqrt
net: resend last three ticks in every packet
render: instance same-type units into one draw call
docs: record ADR for lockstep networking
```

### Pull request checklist

- [ ] Tests pass, clippy clean, formatted
- [ ] No floats and no engine deps added to `redshift-sim`
- [ ] Determinism suite passes if the sim was touched
- [ ] `--bench` still within budget if rendering was touched
- [ ] New content added as data, not Rust
- [ ] An ADR added if a significant decision was made
- [ ] `TODO.md` updated

## Code style

- Rust 2024 edition, `rustfmt` defaults, clippy clean at `-D warnings`.
- Public items in `redshift-sim` and `redshift-data` carry doc comments.
- Comment *why*, not *what*. Determinism-critical code should say so explicitly, so a future
  reader does not "simplify" it into a bug.
- Prefer clarity over cleverness in the sim. It is the code most likely to be debugged at
  2 a.m. against a desync dump.

## Dependencies

Each dependency is a maintenance, compile-time and binary-size cost. New ones need justification
in the PR description. Dependencies in `redshift-sim` face a high bar and must be
determinism-safe — anything using floats, hashing with random seeds, or threads is disqualified.

## Architecture decisions

Significant decisions get an ADR in [docs/adr/](docs/adr/). Prefer superseding an existing ADR
with a new one over quietly contradicting it. The point is that decisions are made once and
recorded, not re-argued.

## Documentation language

Repository documentation is written in English so the project stays contributable. Issues and
discussion may be in any language.
