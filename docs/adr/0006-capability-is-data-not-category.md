# ADR 0006 — Capability belongs to the unit, never to its category

**Status:** Accepted · **Date:** 2026-08-19

## Context

The engine had begun inferring what a unit can do from what *kind* of unit it
is. Two examples, both real, both in shipped code:

```rust
// What terrain can I cross? — decided by an enum, in Rust.
match self {
    Locomotor::Ship  => terrain == Terrain::Water,
    Locomotor::Hover => matches!(terrain, Terrain::Ground | Terrain::Water),
    Locomotor::Foot | Locomotor::Wheeled | Locomotor::Tracked => terrain == Terrain::Ground,
    Locomotor::Air   => true,
}

// How much room do I take up? — decided by a category string, in Rust.
match def.category.as_str() {
    "infantry" => 0.16,
    "vehicle"  => 0.39,
    ...
}
```

Both read as sensible defaults. Both are actually rules, and putting a rule in
a `match` on a category means every exception to it is a code change.

The original is *full* of exceptions, and they are not incidental — they are
most of what distinguishes one unit from another:

- Infantry cannot cross water. Some infantry can.
- Vehicles cannot cross water. Hovercraft can cross both.
- Ships cannot leave water. A carrier's aircraft leave it constantly.
- Structures do not move. A construction vehicle becomes one, and can become a
  vehicle again.
- Units do not disappear when used. An engineer entering a building does.

Under the current design each of those needs a new enum variant or a new arm in
a match — which contradicts the promise the whole data layer exists to keep:
that adding a unit is a data and art task.

The failure mode is worse than the work. A category-based default is *usually
right*, so the design looks fine for a long time and then produces a roster
where a quarter of the units each needed their own special case, none of which
compose with each other.

## Decision

**A unit's capabilities are declared on the unit. The engine never infers a
capability from a category, a locomotor, or any other label.**

Concretely:

- **Surfaces are data.** A unit declares which surfaces it can cross. The
  locomotor survives as a *movement style* — what it looks like, whether it
  crushes, how it turns — and supplies a default surface set when a unit does
  not override one. An amphibious rifleman is a one-line data change.
- **Physical size is data**, defaulting from the locomotor for the same reason.
- Where a default is convenient, it is a **default the data may override**,
  never a rule the data cannot reach.

Categories keep exactly one job: saying which producer builds a thing. That is
a grouping, not a capability, and it is what `Produces` already matches on.

## Consequences

- The surface rules move out of `TerrainRules` and into the unit's resolved
  stats. Pathfinding asks the unit what it can cross rather than asking its
  locomotor.
- Rules files get slightly longer for ordinary units, since the common case is
  now stated rather than assumed. That is the price of the exceptional case
  being free, and it is the right way round: there are far more exceptional
  units in this game than the categories suggest.
- Several capabilities identified in [08-roster.md](../08-roster.md) become
  expressible without new code — amphibious anything, hover anything, a unit
  that crosses cliffs.
- Others remain genuinely new mechanics rather than new combinations, and are
  still engine work: deploying, being consumed on use, garrisoning, carrying
  aircraft. The test for which is which is now clear — if it is a *choice among
  things the engine already does*, it is data; if it is a thing the engine has
  never done, it is code.

## The rule this leaves behind

When adding a capability, ask: **could a unit reasonably want the opposite of
this?** If yes, it is data. Almost everything about a unit fails that test in
this game, which is why the default has to be data rather than code.
