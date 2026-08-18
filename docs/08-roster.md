# 08 — The world the engine has to hold

## Why this document exists

Content belongs to Phase 5. This is not content — it is the **specification the
engine is built against**, and it belongs to Phase 3.

The trait system's whole promise is that adding a unit is a data and art task.
That promise is only testable against a list of what actually has to exist.
Writing the list is cheap. Discovering in Phase 5 that a third of the roster
each needs its own engine change is not.

It has a second use, which is why it is this detailed: **it is the test
oracle.** Every entry below is a question the engine can be asked. A civilian
that wanders, a bridge that can be cut, an oil derrick that pays its captor —
each is a concrete behaviour to write a test for, long before the art exists.

## Accuracy

**Written from memory, not verified.** Names, numbers and specific behaviours
need checking against the original before any of it is treated as settled. It is
accurate enough to audit the engine against, which is what it is for. Entries
that are guesses are marked ⚠️.

Names are the original's, used to identify what is being described. They are not
the names that ship — see
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md).

---

## 1. Terrain and map features

What a map is made of, beyond flat ground.

| Feature | Behaviour | Engine |
|---|---|---|
| Ground | Buildable, drivable | ✅ |
| Water | Naval only, unless amphibious | ✅ |
| Cliffs / elevation | Blocks ground movement; **units on high ground see further and are harder to hit** ⚠️ | ❌ height is faked with rock |
| Ramps | The only way between elevations | ❌ |
| Shore | Where transports load and unload | ❌ |
| Bridges | Crossable, **destructible, and repairable by an engineer** | ❌ |
| Ore / gems | Regrows from a source over time ⚠️; gems are worth more | ⚠️ no regrowth, no second kind |
| Tunnels | Enterable at one end, exit at another ⚠️ | ❌ |
| Trees, rocks | Block movement; trees burn ⚠️ | ⚠️ blocking only |
| Roads | Cosmetic, or a speed bonus ⚠️ | ❌ |

**The big one is elevation.** It is not decoration: it changes what can be
walked, what can be seen, and what can be shot. Rock currently stands in for it
and blocks everything, which is not the same rule at all.

---

## 2. Neutral and civilian things

An entire category the engine has no concept of. Everything below belongs to
nobody, and that is exactly what makes it interesting.

### Civilians

Men, women and children wandering a town, plus civilian cars. They have idle
walks and reactions, they can be run over and shot, and killing them
deliberately is something a player can choose to do.

For the engine this is a **neutral player** — a side that owns things, is
hostile to nobody, and issues no commands. Wandering is an autonomous behaviour
like the harvester cycle, but purposeless, which is its own small design
question: what does a civilian do when a battle arrives?

### Occupiable structures

Civilian buildings that infantry can garrison, firing from windows. The building
becomes a fortification that has to be cleared rather than merely destroyed, and
whoever is inside dies with it.

Needs: a structure that holds passengers, passengers that fire from inside, a
capacity, and an eviction rule.

### Capturable structures

| Structure | Effect when captured |
|---|---|
| Oil derrick | A one-off payment, then a trickle of income |
| Hospital | Nearby infantry heal ⚠️ |
| Machine shop | Nearby vehicles repair ⚠️ |
| Airport | Grants a support power ⚠️ |
| Radar dome | Reveals part of the map ⚠️ |

Each is captured by an engineer walking in, and each keeps working for whoever
holds it. `Capturable` exists in the catalogue and nothing reads it.

### Tech and hazards

Barrels and crates that explode when shot, and crates that grant money, a free
unit, a heal, or a one-off power when driven over.

---

## 3. Countries

Nine, and the shipped remaster has exactly these — see
[adr/0005-faithful-remaster-scope.md](adr/0005-faithful-remaster-scope.md).

| Side | Country | Unique |
|---|---|---|
| Allied | America | Paratroopers, as a recurring power |
| Allied | Korea | A fast strike aircraft |
| Allied | France | A very long-ranged fixed gun |
| Allied | Germany | A tank destroyer, strong against armour only |
| Allied | Great Britain | A sniper that kills infantry outright |
| Soviet | Russia | A tank with a Tesla weapon |
| Soviet | Iraq | A unit that irradiates ground, denying it |
| Soviet | Libya | A truck that detonates |
| Soviet | Cuba | Suicide bombing infantry |

The pattern is one unique unit or power each, on a shared side roster. The data
layer already expresses `unique_units`, `removes_units` and `modifiers`; none of
it has been exercised.

**Two of these are not units at all.** Paratroopers are a power with a cooldown,
and ground denial is a persistent area effect. Both need mechanics that do not
exist.

---

## 4. Tech tree

The shape both sides share:

```
Construction Yard ──┬─▶ Power Plant ─────────▶ (everything else needs power)
                    ├─▶ Ore Refinery ────────▶ income, and a free harvester
                    ├─▶ Barracks ────────────▶ infantry
                    ├─▶ War Factory ─────────▶ vehicles
                    ├─▶ Naval Yard ──────────▶ ships        ⚠️ must touch water
                    ├─▶ Radar / Air HQ ──────▶ minimap, aircraft
                    ├─▶ Battle Lab ──────────▶ advanced units, superweapons
                    ├─▶ Service Depot ───────▶ repairs vehicles, sells them
                    └─▶ defences
```

Three properties in that diagram the engine does not model:

- **A structure that unlocks rather than produces.** A Battle Lab makes nothing.
  Prerequisites cover this and nothing has exercised it.
- **A structure with its own placement rule.** A Naval Yard must touch water;
  `Map::can_place` knows only terrain and occupancy.
- **A structure that acts on units.** A Service Depot repairs and sells.

---

## 5. Infantry

| Role | Allied | Soviet | Engine |
|---|---|---|---|
| Basic | GI — **deploys into a static, stronger stance** | Conscript | ❌ deploy |
| Anti-armour | Guardian GI (deploys) | Tesla Trooper — **also powers a Tesla Coil** ⚠️ | ❌ deploy, ❌ charging a structure |
| Anti-air | — | Flak Trooper | ⚠️ needs air targets |
| Scout | Attack Dog — **detects spies, kills infantry outright** | Attack Dog | ⚠️ detection ✅, instant kill ❌ |
| Engineer | Engineer | Engineer | ❌ capture, ❌ repair, ❌ consumed on use |
| Air | Rocketeer — flying infantry | — | ❌ |
| Demolition | Navy SEAL, Tanya — **charges that destroy a building outright** | Crazy Ivan — **places bombs on anything, including units** | ❌ |
| Infiltration | Spy — **disguised; effect depends on the building entered** | — | ❌ disguise, ❌ per-building effects |
| Special | Chrono Legionnaire — **erases a target over time; teleports** | Yuri — **mind control**; Desolator — **ground denial** | ❌ all three |
| Hero | Tanya | Boris — **calls an airstrike** | ❌ |

The engineer is worth calling out because it is three mechanics at once, and the
user's summary of it is exactly right: **it enters a building, repairs or
captures it, and is consumed.** Nothing in the engine can express any of the
three.

---

## 6. Vehicles

| Role | Allied | Soviet | Engine |
|---|---|---|---|
| Main tank | Grizzly | Rhino | ✅ |
| Heavy tank | — | Apocalypse — **two weapons, ground and air** | ❌ multiple weapons |
| Harvester | Chrono Miner — **teleports home when full** | War Miner — **armed** | ✅ base, ❌ teleport, ❌ armed harvester |
| Base vehicle | MCV | MCV | ❌ **deploys into a Construction Yard, and back** |
| Transport | IFV — **weapon changes with its passenger** | Flak Track | ❌ |
| Assault transport | Battle Fortress — **passengers fire from inside** | — | ❌ |
| Artillery | Prism Tank — **beams chain between targets** ⚠️ | V3 Launcher — **slow visible missile** | ❌ projectiles, ❌ chaining |
| Anti-infantry | Robot Tank — **hovers, immune to mind control** | Terror Drone — **enters a vehicle and kills it from inside** | ❌ |
| Deploying | — | Siege Chopper — **flies, lands, becomes artillery** | ❌ |
| Disguise | Mirage Tank — **looks like a tree** | — | ❌ |
| Air transport | Nighthawk | — | ❌ |
| Demolition | — | Demolition Truck | ❌ |

---

## 7. Aircraft and naval

Both need whole subsystems that do not exist in any form.

**Aircraft** launch from a pad, attack, return, rearm, and relaunch. They do not
use the ground pathfinder, they cannot be attacked by most weapons, and they
crash rather than simply vanishing.

**Naval** needs water pathing that works, transports that load and unload across
a shoreline, and submarines that are cloaked until they fire. An aircraft
carrier is both problems at once — the user's example is exact: **it is a ship
whose aircraft move like air units.**

---

## 8. Superweapons and powers

| | Effect | Shape of the mechanic |
|---|---|---|
| Nuclear Missile | Huge damage in a radius | Charge timer, targeted, delayed |
| Iron Curtain | Makes units invulnerable briefly | Charge timer, targeted, temporary status |
| Chronosphere | Teleports units | Charge timer, two targets |
| Weather Control | Roaming storm | Charge timer, persistent moving effect |
| Paratroopers | Delivers infantry | Recurring power, spawns units |
| Spy Plane | Reveals an area | Recurring power, visibility effect |

Common machinery: a charge timer, a targeting mode in the interface, and effects
that are not "damage in a radius". None of it exists.

---

## 9. The audit is executable

This document used to end in a hand-written list of gaps, which is exactly the
kind of thing that drifts from the code and then misleads. The list lives in
`crates/sim/tests/roster_conformance.rs` now.

Every capability here is a test that builds the thing **through the data layer**,
as a real unit would. A passing test is a capability the engine has. An ignored
test is one it lacks, with the reason attached.

```sh
# what works
cargo test -p redshift-sim --test roster_conformance

# the live gap list, with reasons
cargo test -p redshift-sim --test roster_conformance -- --list | grep ignore
```

Closing a gap means deleting an `#[ignore]`. If a test there ever needs a Rust
change to express a *unit*, ADR 0006 has been violated somewhere.

**As of the last run: 9 capabilities confirmed, 13 gaps.**

Confirmed working, exercised end to end rather than asserted:

- Ordinary infantry cannot cross water, and amphibious infantry can — from one
  line of data, no engine change
- A hovercraft crosses both surfaces; a ship cannot leave the water; aircraft
  cross everything including high ground
- A unit may declare a size that its category would not give it
- A producer builds only its declared categories
- Prerequisites gate the tech tree, and a structure that produces nothing can
  still unlock things

## 10. What the engine cannot do

Consolidated from everything above, ordered by how much each represents.

### Small — a trait and a rule each

1. **Capture** — `Capturable` exists, unread
2. **Repair** — a structure that heals what comes to it
3. **Consumed on use** — the engineer disappearing
4. **Walls** — one-cell structures that connect
5. **Map reveal** — a one-off effect on the visibility layers
6. **Economy modifiers on a structure**
7. **A neutral player** — owns things, commands nothing, hostile to nobody
8. **Instant-kill weapons** — sniper, attack dog
9. **Placement rules per structure** — must touch water

### Medium — real mechanics with interactions

10. **Deploy**, both kinds: unit↔structure, and stance toggling
11. **Garrison** — passengers firing from a building, evicted when it falls
12. **Transports** — loading, unloading, passengers that fire or change the weapon
13. **Projectiles** — shots currently land instantly
14. **Air targeting** — nothing distinguishes an air target from a ground one
15. **Multiple weapons per unit** — anti-ground and anti-air on one chassis
16. **Placed charges** — armed now, detonating later
17. **Temporary status effects** — invulnerable, irradiated, disabled
18. **Wandering civilians** — autonomous, purposeless movement

### Large — each is a subsystem

19. **Elevation** — real height, ramps, and its effect on sight and combat
20. **Aircraft** — basing, rearming, a separate movement model
21. **Naval** — shoreline transports, water as a surface
22. **Superweapons and powers** — timers, targeting modes, novel effects
23. **Mind control** — changing a unit's owner mid-match
24. **Teleportation** — movement without a path
25. **Disguise** — appearing as something else to one side only
26. **Bridges** — destructible terrain that changes connectivity

---

## 11. What follows

**The Phase 3 exit criteria are wrong.** They say a 1v1 skirmish is playable,
which is nearly true, and say nothing about the roster being expressible. Items
1–18 are engine capability rather than content, which makes them Phase 3 work by
any reasonable reading, and none of them are in the plan.

**Two items are foundational and should come first.** Shots land instantly and
nothing distinguishes an air target from a ground one. Everything built on top
of the current combat model has to be revisited when those change, so the longer
they wait the more expensive they get.

**Items 19–26 need a decision, not a schedule.** Some may be Phase 5 work done
alongside the units that need them. Elevation probably is not — it changes the
map format, which everything else is built on.

**And this document needs verifying.** It is a memory dump. Its value is in
having something concrete to check the engine against, and getting it wrong
means building the wrong engine quietly.
