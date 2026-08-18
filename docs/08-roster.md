# 08 — The roster, and what the engine still cannot express

## Why this document exists

Content belongs to Phase 5. This is not content — it is the **requirements
document for the trait catalogue**, and it belongs to Phase 3.

Without it we are building an engine without knowing what it has to be able to
express. The trait system's whole promise is that adding a unit is a data and
art task; that promise is only testable against a list of the units that
actually have to exist. Writing the list is cheap. Discovering in Phase 5 that
half the roster needs new engine work is not.

## A warning about accuracy

**This is a first pass written from memory and is not verified.** Names,
groupings and especially specific behaviours need checking against the original
before any of it is treated as settled. It is accurate enough to audit the
trait catalogue against, which is what it is for.

Names here are the original's, used to identify what is being described. They
are not the names that ship — see
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md).

## Shape of the tech tree

Both sides share a structure, which is the useful part:

```
Construction Yard ──┬─▶ Power Plant
                    ├─▶ Ore Refinery ──▶ (ore income)
                    ├─▶ Barracks ───────▶ infantry
                    ├─▶ War Factory ────▶ vehicles
                    ├─▶ Naval Yard ─────▶ ships          (needs adjacent water)
                    ├─▶ Radar / Air HQ ─▶ radar, aircraft
                    ├─▶ Battle Lab ─────▶ advanced units, superweapons
                    ├─▶ Service Depot ──▶ (repairs vehicles)
                    └─▶ defences
```

Three things in that diagram the engine does not currently model at all:

- **A producer that unlocks rather than produces.** A Battle Lab makes nothing;
  it exists so that other things become available. Prerequisites already cover
  this, and it works — but nothing has been built that uses it.
- **A structure with a placement rule of its own.** A Naval Yard must touch
  water. `Map::can_place` knows about terrain and occupancy and nothing else.
- **A structure that acts on units rather than producing them.** A Service
  Depot repairs. There is no trait for it.

## Rosters

Grouped by role rather than by side, since the two sides mirror each other and
the roles are what the engine has to express.

### Structures

| Role | Allied | Soviet | Engine support |
|---|---|---|---|
| Base | Construction Yard | Construction Yard | ✅ |
| Power | Power Plant | Tesla Reactor, Nuclear Reactor | ✅ |
| Economy | Ore Refinery, Ore Purifier | Ore Refinery | ✅ refinery, ❌ purifier bonus |
| Infantry | Barracks | Barracks, Cloning Vats | ✅ barracks, ❌ free duplicate |
| Vehicles | War Factory | War Factory, Industrial Plant | ✅ factory, ⚠️ cost modifier |
| Naval | Naval Yard | Naval Yard | ❌ water-adjacent placement |
| Air | Airforce Command HQ | — | ❌ aircraft basing |
| Tech | Battle Lab | Battle Lab | ✅ via prerequisites |
| Repair | Service Depot | Service Depot | ❌ |
| Vision | Spy Satellite Uplink | Radar Tower | ❌ map reveal |
| Counter-vision | Gap Generator | — | ❌ imposing fog on an enemy |
| Defence | Pillbox, Patriot, Prism Tower | Sentry Gun, Flak Cannon, Tesla Coil | ✅ armed structures |
| Wall | Wall | Wall | ❌ connecting segments |
| Superweapon | Chronosphere, Weather Control | Iron Curtain, Nuclear Missile | ❌ |

### Infantry

| Role | Allied | Soviet | Engine support |
|---|---|---|---|
| Basic | GI | Conscript | ✅ |
| Anti-armour | Guardian GI | Tesla Trooper | ✅ |
| Anti-air | — | Flak Trooper | ✅ (needs air targeting) |
| Scout | Attack Dog | Attack Dog | ⚠️ detects spies |
| Capture | Engineer | Engineer | ❌ capture |
| Air | Rocketeer | — | ❌ flying infantry |
| Demolition | Navy SEAL, Tanya | Crazy Ivan | ❌ placed charges |
| Infiltration | Spy | — | ❌ disguise, ❌ infiltration effects |
| Special | Chrono Legionnaire | Yuri, Desolator | ❌ teleport, ❌ mind control |
| Hero | Tanya | Boris | ❌ (composition of the above) |

### Vehicles

| Role | Allied | Soviet | Engine support |
|---|---|---|---|
| Main tank | Grizzly | Rhino | ✅ |
| Heavy tank | — | Apocalypse | ✅ |
| Harvester | Chrono Miner | War Miner | ✅ miner, ❌ teleport home |
| Transport / IFV | IFV, Battle Fortress | Flak Track | ❌ passenger-dependent weapon, ❌ firing ports |
| Artillery | Prism Tank | V3 Rocket Launcher | ⚠️ needs projectiles |
| Anti-infantry | Robot Tank | Terror Drone | ❌ parasite behaviour |
| Deploying | — | Siege Chopper | ❌ deploy |
| Base | MCV | MCV | ❌ deploy into a structure |
| Disguise | Mirage Tank | — | ❌ |
| Air transport | Nighthawk | — | ❌ |

### Aircraft and naval

Both need engine work that does not exist in any form: aircraft have to launch
from a base, attack, and return to rearm, and naval units need water pathing
that works, transports that load and unload across a shoreline, and submarines
that are cloaked until they fire.

## What this audit actually found

The catalogue has twenty traits and covers the **shape** of the game — things
that move, shoot, cost money, draw power, gather ore, get built and get seen.
That is genuinely most of a match.

What it does not cover is the part that makes the original distinctive. In
rough order of how much engine work each represents:

**Small — a trait and a rule each**

1. `Capturable` exists and nothing reads it. Engineers need it.
2. Repair, as a structure that heals vehicles that come to it.
3. Walls, as a structure that occupies one cell and connects to neighbours.
4. Map reveal, as a one-off effect.
5. Economy modifiers on a structure (Ore Purifier, Industrial Plant) — the
   faction `Modifier` machinery is close to this already.

**Medium — new mechanics with real interactions**

6. **Deploy**, both kinds: a unit that becomes a structure (MCV) and a unit
   that toggles a stance (GI, Siege Chopper).
7. **Transport that matters** — loading, unloading, and passengers that shoot
   from inside or change the vehicle's weapon.
8. **Placed charges** — a unit that arms something that detonates later.
9. **Projectiles**. Shots currently land instantly. Artillery, missiles and
   anything with travel time need real ones.
10. **Air targeting**. Nothing distinguishes an air target from a ground one.

**Large — each is its own subsystem**

11. **Aircraft**: basing, rearming, and a movement model that is not the ground
    pathfinder.
12. **Naval**: shoreline transports, and water as a first-class surface rather
    than an obstacle.
13. **Superweapons and support powers**: a charge timer, a targeting mode, and
    effects that are not "damage in a radius".
14. **Mind control**: changing a unit's owner, with all that implies for
    ownership checks, vision and the state hash.
15. **Chrono teleport**: moving a unit without a path, which every assumption
    in the movement code is currently against.
16. **Disguise**: a unit that appears to be something else to one side and not
    the other.

## What follows from this

Three things, in order.

**The Phase 3 exit criteria are wrong.** They say a 1v1 skirmish is playable,
which is nearly true, and say nothing about the roster being expressible. Items
1–10 above are Phase 3 work by any reasonable reading — they are engine
capability, not content — and they are not in the plan.

**Items 11–16 need a decision, not a schedule.** Each is a subsystem. Some may
turn out to be Phase 5 work done alongside the units that need them; some, like
projectiles and air targeting, are foundational enough that everything built on
top of the current instant-hit model would have to be revisited.

**This document needs verifying against the original before it is trusted.**
It is a memory dump, and its value is entirely in having something concrete to
check the engine against. Getting the roster wrong here means building the
wrong engine, quietly.
