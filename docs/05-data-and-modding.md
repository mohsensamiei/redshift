# 05 — Data and Modding

The project goal "add a new country" must be a data-and-art task, not a programming task. That
constrains the design: **no unit, building, weapon or faction may be defined in Rust code.**

## Format

Rules live in RON (Rusty Object Notation) files under `rules/`, parsed by `redshift-data` into
typed structs. RON is chosen over TOML/JSON because it maps directly onto Rust enums and nested
structs, which matters a great deal for the trait lists below.

```
rules/
  units/          one file per unit
  buildings/      one file per structure
  weapons/        weapons and projectiles
  armour.ron      armour classes and the damage multiplier table
  factions/       one file per country
  tech.ron        prerequisites and tech tree
```

Files are merged at load time into a single `Rules` object. Loading validates cross-references
(a unit naming a weapon that does not exist is an error at load, not a crash mid-match) and
computes a hash used by the network layer to confirm all peers agree on the rules.

## Composition over inheritance

Entities are defined as a **list of traits**, not as a subclass of a base type. This is the same
idea OpenRA uses, and it is what makes new content cheap: a novel unit is a novel *combination*
of existing traits, requiring no new code.

```ron
// rules/units/heavy_tank.ron
(
    id: "heavy_tank",
    name_key: "unit.heavy_tank",
    cost: 900,
    build_time: 45,
    prerequisites: ["war_factory"],
    model: "units/heavy_tank.glb",
    traits: [
        Health(( max: 400, armour: "heavy" )),
        Mobile(( speed: 4.5, turn_rate: 90, locomotor: Tracked )),
        Armed(( weapon: "120mm", turret: true )),
        Vision(( range: 6 )),
        Crushes(( classes: ["infantry"] )),
        Selectable(( priority: 2 )),
    ],
)
```

Adding `Cloakable`, `Amphibious` or `MindControllable` to that list changes behaviour with no
Rust change. Adding a genuinely *new* trait is the only case that needs code — and that is the
correct boundary.

### Trait catalogue (initial)

Movement `Mobile` `Amphibious` `Hover` `Flying` `Immobile` ·
Combat `Armed` `Health` `Armour` `SelfHealing` `Explodes` ·
Perception `Vision` `Detector` `Cloakable` ·
Economy `Harvester` `Refinery` `PowerPlant` `PowerDrain` `ProducesUnits` ·
Structure `Buildable` `Capturable` `Repairable` `Bib` ·
Special `Crushes` `Transport` `Deployable` `MindControllable` `Veterancy`

The list grows as phases land. Each trait is a small, independently testable piece of sim logic.

## Damage model

Faithful to the original: a weapon has a **warhead**, armour has a **class**, and a lookup table
gives the multiplier. This is what makes rock-paper-scissors counterplay work, and it stays pure
data:

```ron
// rules/armour.ron
(
    classes: ["none", "light", "heavy", "concrete", "air"],
    table: {
        "small_arms":   { "none": 100, "light": 60, "heavy": 10, "concrete": 5,  "air": 0   },
        "ap_shell":     { "none": 40,  "light": 90, "heavy": 100,"concrete": 60, "air": 0   },
        "explosive":    { "none": 90,  "light": 75, "heavy": 50, "concrete": 100,"air": 0   },
        // values are percentages; illustrative, to be tuned in Phase 3
    },
)
```

## Factions

A country is a small overlay on a side's shared roster — one unique unit or structure, one
passive advantage, and an identity. This mirrors how the original handled countries and keeps
the work per faction to roughly one model plus a few lines of data.

```ron
// rules/factions/example.ron
(
    id: "example",
    name_key: "faction.example",
    side: "allied",              // shares the allied tech tree
    colour: (30, 90, 200),
    unique_units: ["prototype_walker"],
    // removes a unit the side otherwise gets, if the unique replaces it
    removes_units: [],
    modifiers: [
        UnitCost(( unit: "gi", multiplier: 0.9 )),
        BuildSpeed(( category: "defence", multiplier: 1.15 )),
    ],
    voice_set: "example",
)
```

Adding a new country therefore means: one RON file, one model, one voice set, one flag. No Rust.
That is the goal, stated as a testable property — see the Phase 5 exit criteria in
[07-roadmap.md](07-roadmap.md).

## Localisation

No user-visible string is hard-coded. Data files reference keys (`unit.heavy_tank`) resolved
against `locale/<lang>.ron`. English and Persian are the initial targets.

## Rules hashing and multiplayer

`Rules` produces a hash at load. Peers exchange it during the handshake; a mismatch is refused
with a clear message rather than allowed to desync three minutes into the match. This also means
a player cannot quietly buff their own units in a multiplayer game.

## Hot reload

In development builds, editing a rules file reloads it and restarts the current skirmish. Balance
iteration should be seconds, not a recompile. Disabled in release builds and always disabled
during network matches.

## Maps

Maps are data too, in their own format (Phase 3): terrain heightmap, tile types, resource
placement, starting positions, and optional trigger scripts. A map may carry local rule overrides
for scenario purposes, but overrides are disabled in ranked/online matches.
