# 02 — Simulation

The simulation is the part of this project that is genuinely hard to get right, and the part
where mistakes are most expensive to discover late. Read this before touching `redshift-sim`.

## Why determinism is non-negotiable

Multiplayer uses **deterministic lockstep**: peers exchange only player *commands*, never unit
positions. Each peer runs the full simulation locally and must arrive at an identical result.

The payoff is enormous — bandwidth is a few hundred bytes per second regardless of army size,
replays are nearly free, and there is no server-side game logic to host. The cost is a single
strict requirement:

> Given the same initial state and the same command sequence, every peer must produce a
> **bit-identical** world state, on every platform, forever.

If that ever fails, the match desyncs: two players see different games. It usually manifests
minutes after the actual divergence, which is why the discipline below matters more than any
optimisation.

## Fixed-point arithmetic

Floating point is banned in the sim because it is not reliably reproducible across compilers,
optimisation levels, and architectures — and we explicitly target both ARM (Apple Silicon) and
x86. Instead:

```rust
/// Fixed-point scalar: i32 with 16 fractional bits.
/// 1.0 == one map cell. Range ±32768 cells, resolution 1/65536 of a cell.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fx(i32);

pub const FX_ONE: Fx = Fx(1 << 16);
```

Rules:

- All multiplication and division goes through `i64` intermediates, then shifts back. Never
  multiply two `Fx` as `i32` — it overflows silently at surprisingly small values.
- `sqrt` is an integer Newton iteration with a fixed iteration count. No `f32::sqrt`.
- `sin`/`cos` come from a precomputed table indexed by a fixed-point angle (`u16` binary
  angle: full turn = 65536). Never `f32::sin`.
- Distance comparisons prefer squared distance to avoid `sqrt` entirely.
- Division by zero is a bug, not a special case. Assert in debug.

The `Fx` type deliberately does **not** implement `From<f32>`. Converting from a float is only
legal in the renderer, which converts the other way.

## Randomness

One generator, in the sim state, seeded from the match seed:

```rust
pub struct SimRng { state: u64 }   // PCG-XSH-RR or xoshiro; explicit, not `rand`'s default
```

- Never `rand::thread_rng()` or any OS entropy inside the sim.
- The RNG is advanced only by sim code, in tick order. A renderer that wants jitter for a
  particle effect uses its own separate, non-sim RNG.
- The number of RNG draws per tick must not depend on anything non-deterministic — for example,
  never draw inside a loop over a `HashMap`.

## Collections

| Forbidden in sim | Use instead | Why |
|---|---|---|
| `HashMap` / `HashSet` iteration | `BTreeMap` / `BTreeSet` / `Vec` | Hash iteration order is randomised per process |
| `sort_unstable_by` on a partial order | `sort_by` with a total order ending in entity id | Ties must break identically everywhere |
| `Instant::now()`, `SystemTime` | the tick counter | Wall-clock differs per machine |
| `rayon` / thread pools | single-threaded loops | Completion order varies |

Using a `HashMap` for *lookup only* (never iterated) is acceptable but discouraged; a slotmap
with stable indices is usually better and faster.

## Entity model

A slotmap-style arena with generational indices:

```rust
pub struct EntityId { index: u32, generation: u32 }
```

Entities are stored in dense, index-parallel arrays (a hand-rolled ECS-lite). We deliberately do
**not** use Bevy's ECS for the simulation: its scheduler parallelises systems, which is exactly
what we cannot have, and it would couple the sim to the engine. Bevy's ECS is used freely on the
rendering side.

Iteration is always in index order. New entities append; destroyed entities free their slot for
reuse with a bumped generation. Slot reuse order is itself deterministic (a free-list, in order).

## The tick

```rust
pub fn tick(&mut self, commands: &[Command]) {
    // 1. Apply commands. Already ordered by (tick, player_id, sequence) by the net layer.
    // 2. Production, power, economy.
    // 3. Pathfinding requests (bounded budget per tick).
    // 4. Movement.
    // 5. Targeting and combat.
    // 6. Damage resolution, deaths, cleanup.
    // 7. Fog of war update.
    // 8. Victory conditions.
    self.tick += 1;
}
```

Each phase runs to completion for all entities before the next begins. No phase may observe a
partially-updated later phase. This ordering is part of the game's contract — changing it changes
behaviour and invalidates old replays.

**Tick rate is 20 Hz.** Low enough to keep lockstep bandwidth and latency tolerance comfortable,
high enough that the game feels responsive for this genre. Rendering interpolates at 60 Hz.

## Pathfinding

- Grid A\* for individual units; flow fields for large group moves.
- Must be deterministic: a fixed tie-break rule in the open set (lowest `f`, then lowest cell
  index), never a hash-ordered container.
- **Bounded per tick.** Pathfinding is the main CPU cost in an RTS. A fixed budget of node
  expansions per tick is spent in entity-id order; requests that do not finish carry over to the
  next tick. This keeps tick cost stable and — crucially — keeps it *deterministic*, because the
  budget is in node counts, not milliseconds.

Never budget by elapsed time. Time-based cutoffs are the single most common way a sim desyncs.

## State hashing

```rust
pub fn state_hash(&self) -> u64
```

A stable hash over all sim state that affects gameplay: entity positions, health, orders,
resources, RNG state, tick number. Deliberately excludes anything cosmetic.

Peers exchange this hash every 20 ticks (once per second). A mismatch stops the match
immediately and writes a diagnostic dump from both sides. See [03-networking.md](03-networking.md).

## Testing

The determinism suite is the most important test in the repository.

| Test | What it proves |
|---|---|
| `replay_roundtrip` | Same seed + same commands ⇒ identical hash at every tick |
| `two_sims_identical` | Two `Sim` instances in one process stay hash-identical for 10k ticks |
| `serialisation_stable` | Save, load, continue ⇒ identical to never having saved |
| `no_float_in_sim` | Lint: the crate contains no `f32`/`f64` |
| `cross_platform_vectors` | Golden hashes recorded in CI on both ARM and x86 match |

The last one is the real safety net: CI runs the same scripted match on macOS/ARM and
Linux/x86 and compares the hash at fixed checkpoints. It catches exactly the class of bug that
is otherwise invisible until two players on different machines try to play.

**Never relax a determinism test to make it pass.** A failure is always a real bug.
