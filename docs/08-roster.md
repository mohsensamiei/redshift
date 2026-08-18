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

Sections 2 through 8 — civilians, tech structures, spy infiltration,
multi-behaviour units, the infantry, vehicle, naval and air rosters,
superweapons and crates — are **researched against public documentation of the
original**. Sources are listed at the end. Individual entries still marked ⚠️
are ones the sources did not settle.

Still outstanding: **section 1 (terrain)** is from memory, and the **Allied
structure table** has not been read yet — the source rate-limited. Allied
figures are assumed to mirror the Soviet ones, which is exactly the kind of
assumption this document exists to stop.

Names are the original's, used to identify what is being described. They are
not the names that ship — see
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md). Names, numbers and specific behaviours
need checking against the original before any of it is treated as settled. It is
accurate enough to audit the engine against, which is what it is for. Entries
that are guesses are marked ⚠️.

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

Men, women and children wandering a town, plus civilian cars. Verified
behaviour, and the part that matters for the engine:

- They are **neutral and passive**. They do not fight, and military units do
  not acquire them as targets. Standing an army next to a crowd starts nothing.
- A player *can* kill them, but only by giving an explicit order. That is the
  distinction the engine has to hold: auto-acquisition skips them, a deliberate
  attack does not.
- Their movement is aimless — a loop of wandering with no goal. Not an AI in
  any meaningful sense, and it should not be built as one.

For the engine this needs a **neutral player**: a side that owns things, issues
no commands, and is hostile to nobody. Everything else follows from that.

### Occupiable structures

Civilian buildings infantry can garrison, firing from windows. The building
becomes a fortification that must be cleared rather than merely destroyed, and
whoever is inside dies with it.

### Tech structures — captured by an engineer

Verified. These are neutral buildings scattered on maps, marked with a yellow
flag, captured by walking an engineer in.

| Structure | Effect | Game |
|---|---|---|
| **Oil Derrick** | A one-off payment on capture, then a steady trickle of income | RA2 |
| **Hospital** | Heals friendly infantry that walk into it | RA2 |
| **Airport** | Grants the paratrooper power | RA2 |
| **Outpost** | Defends with an IFV missile launcher, *and* acts as a service depot | RA2 |
| **Machine Shop** | All your vehicles self-repair, anywhere on the map | YR |
| **Power Plant** | +200 power | YR |
| **Hospital** (YR) | All your infantry self-heal anywhere, rather than having to enter | YR |
| **Secret Lab** | Grants one country-unique unit you could not otherwise build | YR |

Three properties that apply to all of them, and that the engine has no notion
of:

- **They cannot be sold, and need no power.** Ownership rules differ from an
  ordinary structure's.
- **They extend the build radius.** A captured derrick is a forward base.
  Redshift has a build radius and only structures the player *built* anchor it.
- **The Secret Lab grants a unit chosen from a fixed list** — demolition truck,
  desolator, grand cannon, psi commando, sniper, tank destroyer, terrorist,
  tesla tank. That is a country-unique unit arriving by a route other than
  being that country.

---

## 2b. Spy infiltration

Verified, and considerably richer than "infiltration effects" suggested. Each
building gives a different thing, and the spy has to reach a *specific* kind of
building to get it.

| Infiltrated | Effect |
|---|---|
| **Barracks** | All infantry you produce gain a rank. Does not stack |
| **War Factory** | All vehicles and aircraft you produce gain a rank. Does not stack |
| **Power Plant** | The victim loses power for about a minute |
| **Ore Refinery** | Steals 20% of the victim's funds |
| **Battle Lab** | Unlocks a commando built from the *victim's* technology |

The Battle Lab case is the interesting one. What you get depends on whose lab
it was: an Allied lab gives a Chrono Commando, a Soviet lab a Chrono Ivan, a
Yuri lab a Psi Commando. So infiltration is not one effect with a target — it
is a table keyed on what was infiltrated.

The veterancy effects are worth noting separately because they are **persistent
production modifiers**, not one-off events: everything you build from then on
arrives promoted, including units delivered by paradrop or released from a
destroyed building.

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

## 4. Structures and the tech tree

Soviet costs, power and prerequisites from the source listed at the end. The
Allied side mirrors it; its exact figures are still to be confirmed.

| Structure | Cost | Power | Needs | Notes | Engine |
|---|---|---|---|---|---|
| Construction Yard | 3000 | 0 | — | Built by deploying an MCV | ❌ deploy |
| Tesla Reactor | 600 | **+150** | — | | ✅ |
| Ore Refinery | 2000 | −50 | Reactor | **Comes with a free miner** | ❌ a building that spawns a unit |
| Barracks | 500 | −10 | Reactor | | ✅ |
| War Factory | 2000 | −25 | Refinery, Barracks | | ✅ |
| Naval Shipyard | 1000 | −20 | Refinery | **Must be placed in water**; ships are **repaired** here | ❌ placement rule, ❌ repair |
| Radar Tower | 1000 | −50 | Refinery | **Stops working when power is short** | ⚠️ low power slows production; it does not disable |
| Service Depot | 800 | −20 | War Factory | Repairs vehicles; **removes a Terror Drone** | ❌ |
| Battle Lab | 2000 | −100 | Factory, Radar | Unlocks, produces nothing | ✅ prerequisites |
| Nuclear Reactor | 1000 | **+1000** | Battle Lab | **Explodes with fallout when destroyed** | ❌ death effect |
| Cloning Vats | 2500 | −200 | Battle Lab | **Duplicates every infantry you train, free**; sells units for a refund; **one per player** | ❌ all three |
| Fortress Wall | 100 | 0 | Barracks | **Four sections placed at once** | ❌ walls |
| Sentry Gun | 500 | **0** | Barracks | Anti-infantry. **Needs no power** | ✅ |
| Flak Cannon | 1000 | −50 | Barracks | Anti-air; **shoots down missiles**; **switches off in low power** | ❌ interception, ❌ disable |
| Tesla Coil | 1500 | −75 | Radar | **Troopers charge it** for more range and power; **three charged troopers make it work without power at all** | ❌ |
| Psychic Sensor | 1000 | −50 | Battle Lab | **Shows the orders enemy units have been given**; reveals spies | ❌ |
| Iron Curtain | 2500 | −200 | Battle Lab | Superweapon. **One per player** | ❌ |
| Nuclear Missile Silo | 5000 | −200 | Battle Lab | Superweapon. **One per player** | ❌ |

Five mechanics in that table alone that were not on any earlier list:

- **A building that comes with a unit.** A refinery arrives with a miner.
- **A building limited to one per player**, which is different from a unit
  build limit and applies to three separate structures.
- **Structures that switch off in low power** rather than merely slowing.
  Redshift models low power as a production slowdown; the original *disables*
  radar and flak cannons outright.
- **Structures that explode when destroyed**, with lasting ground effects.
- **A defence that a unit can charge**, and that becomes independent of the
  power grid once charged enough. That is a unit modifying a structure, which
  nothing in the engine can express.

### The shape of the tree

```
Construction Yard ──▶ Reactor ──┬─▶ Refinery ──┬─▶ Barracks ─┬─▶ War Factory
                                │              │             ├─▶ walls, sentry, flak
                                │              ├─▶ Naval Yard  (must touch water)
                                │              └─▶ Radar ─────┬─▶ Tesla Coil
                                │                             └─▶ Battle Lab ──┬─▶ Nuclear Reactor
                                └─▶ (everything needs power)                   ├─▶ Cloning Vats
                                                                               ├─▶ Psychic Sensor
                                                                               └─▶ superweapons
```

Note that the War Factory needs **both** a Refinery and Barracks, and the
Battle Lab needs **both** a Factory and Radar. Prerequisites are a set, not a
chain, which Redshift already handles.

## 4b. Units that do more than one thing

The point this document previously glossed over. A great many units are not
"a unit with a weapon" — they are several capabilities at once, and which one
applies depends on what they are pointed at.

**Tanya** is the clearest case, and needs four separate mechanics:

| Capability | Detail |
|---|---|
| Pistols | **Instantly kill** any infantry she can reach. Useless against vehicles |
| C4 | Destroys **structures** and **naval units** outright. In YR, ground vehicles too |
| Swimming | She crosses water — amphibious infantry |
| Build limit | Only one at a time. Two with a Cloning Vat |

Four capabilities, and the first two are chosen **by what she is aimed at**:
infantry get shot, buildings get charges. That is not "two weapons"; it is two
actions with different valid targets and different effects.

A **Navy SEAL** is nearly the same unit with one difference — no anti-vehicle
C4 in RA2 — which is exactly the kind of near-duplicate a trait system should
express as a data difference and nothing else.

**The Engineer** is three mechanics in one:

- Enters a building and **captures** it, if it is an enemy's or neutral
- Enters a damaged friendly building and **fully repairs** it
- Is **consumed** either way

**The IFV** changes weapon by passenger, and this is much larger than it
sounds: RA2 has **24 turret modes**, and Yuri's Revenge adds four more. An
engineer inside turns it into a repair vehicle. So the vehicle's weapon is a
function of its cargo, resolved at runtime.

**Units that deploy** are a category rather than an exception:

| Unit | Deployed form |
|---|---|
| MCV | Becomes a Construction Yard, and back again |
| GI, Guardian GI | A static, stronger emplacement. Cannot move while deployed |
| Siege Chopper | Lands and becomes artillery |
| Desolator | Static, irradiating the ground around it |
| Yuri | A wider mind-control radius |

**Tanks crush infantry** while also firing, which is the plainest example of
two capabilities running at once rather than one being chosen.

The engine consequence, stated once: **a unit's capability is a list of
actions, each with its own valid targets and its own effect** — not a weapon
slot. This is the same shape as ADR 0006 and probably wants its own decision
record before anything is built on it.

## 5. Infantry

Costs and prerequisites from the source listed at the end. The engine column
says whether Redshift can express the behaviour at all.

| Unit | Side | Cost | Needs | What it actually does | Engine |
|---|---|---|---|---|---|
| GI | A | 200 | — | **Deploys** into a machine-gun emplacement: more range and power, cannot move. **Can garrison** civilian buildings | ❌ deploy, ❌ garrison |
| Conscript | S | 100 | — | Basic, cheap, slow. Also garrisons | ❌ garrison |
| Engineer | both | 500 | — | **Captures** enemy and neutral buildings, **repairs** friendly ones **and bridges**, **defuses bombs**, and is **consumed** | ❌ all of it |
| Attack Dog | both | 200 | — | Kills infantry outright; **detects spies**; useless against vehicles and structures | ❌ instant kill, ❌ see through disguise |
| Rocketeer | A | 600 | Air HQ | Jet-pack infantry: flies, hits air and ground | ❌ flying infantry |
| Sniper | GB | 600 | Air HQ | Kills infantry with **one shot** at long range | ❌ instant kill |
| Navy SEAL | A | 1000 | Air HQ | Rifle plus **C4**; **crosses land and water** | ❌ charges, ✅ amphibious |
| Spy | A | 1000 | Battle Lab | **Disguised** as enemy infantry; infiltrates for a per-building effect | ❌ disguise, ❌ infiltration |
| Tanya | A | 1000 | Battle Lab | **One-shot kills** infantry, **swims**, **C4** destroys buildings and ships | ❌ instant kill, ❌ charges |
| Chrono Legionnaire | A | 1500 | Battle Lab | **Erases** a target progressively; **interrupting it undoes the erasure**; teleports | ❌ |
| Tesla Trooper | S | 600 | — | **Immune to being crushed**; can **charge a Tesla Coil** to extend its range and power | ❌ crush immunity, ❌ charging a structure |
| Flak Trooper | S | 300 | Radar | Anti-air and anti-vehicle, **splash** | ✅ since air targeting landed |
| Terrorist | Cuba | 200 | Radar | Suicide explosion with splash | ❌ |
| Desolator | Iraq | 600 | Radar | Melts infantry; **deployed, irradiates ground and makes it impassable** | ❌ deploy, ❌ terrain-altering effect |
| Crazy Ivan | S | 600 | Radar | **Places dynamite** on structures, units **and bridges** | ❌ |
| Yuri / Psi-Corps | S | 1200 | Battle Lab | **Mind control**; a psychic blast that kills surrounding infantry | ❌ |
| Chrono Commando | A | 2000 | Spy in Allied lab | SEAL plus teleport. **Cannot swim** | ❌ |
| Chrono Ivan | S | 1000 | Spy in Allied lab | Ivan plus teleport | ❌ |
| Yuri Prime | S | 2000 | Spy in Soviet lab | Longer-ranged mind control. **One per player** without a Cloning Vat | ❌ build limit |

## 6. Vehicles

| Unit | Side | Cost | Needs | What it actually does | Engine |
|---|---|---|---|---|---|
| Grizzly | A | 700 | — | Main tank. Faster and cheaper than a Rhino; **crushes infantry** | ❌ crushing |
| Rhino | S | 900 | — | Main tank. More armour and range, slower | ✅ |
| Flak Track | S | 500 | — | Fast anti-air **and** a transport for five | ❌ transport |
| IFV | A | 600 | — | **Anti-air by default**; weapon changes with its passenger — 24 modes; an engineer makes it a repair vehicle | ❌ |
| Terror Drone | S | 500 | — | **Jumps into an enemy vehicle** and dismantles it from inside; behaves like an attack dog against infantry | ❌ |
| V3 Launcher | S | 800 | Radar | Long range. **Its rocket can be shot down in flight** | ❌ interception |
| Tesla Tank | Russia | 1200 | Radar | **Fires over obstacles** | ❌ indirect fire |
| Demolition Truck | Libya | 1500 | Radar | Nuclear charge, detonates **on destruction or on impact** | ❌ |
| Chrono Miner | A | 1400 | Refinery | **Teleports home** when full; can be ordered to teleport as an escape | ❌ |
| War Miner | S | 1400 | Refinery | Armed, higher capacity | ❌ armed harvester |
| Tank Destroyer | Germany | 1000 | Air HQ | Very strong against vehicles, weak against everything else | ✅ armour table does this |
| Mirage Tank | A | 1000 | Battle Lab | Looks like a **tree** when still. **Can fire while disguised**, and firing drops it | ❌ |
| Prism Tank | A | 1200 | Battle Lab | Beam **reflects onto further targets**; weak armour, poor against vehicles | ❌ chaining |
| Battle Fortress | A | — | Battle Lab | Five passengers **firing from inside**; crushes even things normally uncrushable | ❌ |
| Robot Tank | A | — | Battle Lab | **Hovers**, so crosses water; **immune to mind control** — no driver | ✅ hover, ❌ immunity |
| Apocalypse | S | 1750 | Battle Lab | **Twin cannon vs ground and twin missiles vs air**; slow | ❌ two weapons |
| MCV | both | 3000 | Service Depot | **Becomes a Construction Yard**, and back | ❌ deploy |
| Siege Chopper | S | — | — | Flies, **lands and becomes artillery** | ❌ |
| Grand Cannon | France | — | — | Fixed, very long range | ✅ armed structure |

## 7. Naval and air

| Unit | Side | Cost | Needs | What it does | Engine |
|---|---|---|---|---|---|
| Amphibious Transport | both | 900 | — | **Twelve slots**, infantry and vehicles, **crosses land and water**, unarmed | ❌ transport, ✅ amphibious |
| Destroyer | A | 1200 | — | Ship-to-ship and shore bombardment; **detects submerged units** | ❌ detection at sea |
| Aegis Cruiser | A | 1000 | Air HQ | Anti-air **and anti-missile** — it shoots down projectiles | ❌ interception |
| Dolphin | A | 500 | Battle Lab | Submerged, sonic weapon, anti-submarine | ❌ |
| Aircraft Carrier | A | 2000 | Battle Lab | Launches **three Hornets** that attack, **land, rearm, and go again**; lost aircraft are replaced | ❌ |
| Typhoon Sub | S | 1000 | — | Submerged; **becomes visible when it attacks or is damaged** | ❌ naval cloak |
| Sea Scorpion | S | 600 | Radar | Hits ground, sea and air; **anti-missile system** | ❌ |
| Giant Squid | S | 1000 | Battle Lab | **Grabs and crushes** a ship; visible when damaged | ❌ |
| Dreadnought | S | 2000 | Battle Lab | Long-range missiles at ground targets. **Shootable down in flight** | ❌ |
| Nighthawk | A | 1000 | — | Carries five infantry; **invisible to radar** | ❌ |
| Harrier | A | 1200 | — | Attacks, returns to a pad, **rearms**, relaunches | ❌ |
| Black Eagle | Korea | 1200 | — | A tougher Harrier | ❌ |
| Kirov | S | 2000 | Battle Lab | Very tough bomber; **can only hit what is directly below it** | ❌ |

Two things this table makes obvious that the prose did not.

**Projectile interception is a real, recurring mechanic**, not an exotic one:
the Aegis Cruiser and Sea Scorpion exist largely to shoot down missiles, and
the V3 and Dreadnought exist to fire missiles that can be shot down. Redshift
already has projectiles in flight and in the state hash, so this is closer than
most of the list.

**Submersion is a third visibility state**, beside cloaked and fogged: a
submarine is hidden until it attacks *or is damaged*, and specific units detect
it. That is not quite the cloak rule already implemented, and assuming it is
would be wrong in a way that only shows up in naval play.

## 8. Superweapons, powers and crates

Researched.

| | Effect | Shape of the mechanic |
|---|---|---|
| **Nuclear Missile** | Heavy damage in a radius | Charge timer, targeted, delayed arrival |
| **Iron Curtain** | Makes vehicles and structures in a small area **invulnerable for about 50 seconds** — and **kills any infantry** caught in it. Also cures a Terror Drone infection | Timed status on several units at once, with a second effect on a different unit type |
| **Chronosphere** | Teleports units from one place to another. **Cannot move infantry** | Two targets, movement without a path, a per-type restriction |
| **Weather Control** | A storm that roams and damages | Persistent moving effect |
| **Paratroopers** | Delivers infantry from off-map | Recurring power that spawns units |
| **Spy Plane** | Reveals an area | Recurring power, visibility effect |

Two details worth pulling out, because they are the kind that get missed:
the Iron Curtain **kills the infantry it covers** rather than protecting them,
and the Chronosphere **cannot teleport infantry at all**. Both are per-unit-type
rules attached to an effect, not to a unit.

### Crates

Neutral pickups scattered on the map, collected by driving over them.

| Crate | Effect |
|---|---|
| Money | Credits |
| Veterancy | Promotes units **in an area**, not just the one that collected it |
| Armour, firepower, speed | Permanent upgrades, again area-of-effect |
| Full heal | Restores all your units and structures, and **removes Terror Drone infections** |
| Free vehicle | Any ground vehicle, including an MCV or a miner. Never air or naval |
| Map reveal | Reveals the map, except ground hidden by a gap generator |
| Explosive | Damages whatever opened it |

The upgrades are worth noting: they are **permanent per-unit modifiers applied
in an area**, which is a third shape again beside "damage now" and "a standing
modifier on a player".

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

**As of the last run: 16 capabilities confirmed, 25 gaps.**

The gap count went *up* after research, twice, which is the point of doing it.
Thirteen of those twenty-five were invisible until the mechanics were looked up
rather than recalled.

Confirmed working, exercised end to end rather than asserted:

- Ordinary infantry cannot cross water, and amphibious infantry can — from one
  line of data, no engine change
- A hovercraft crosses both surfaces; a ship cannot leave the water; aircraft
  cross everything including high ground
- A unit may declare a size that its category would not give it
- A producer builds only its declared categories
- Prerequisites gate the tech tree, and a structure that produces nothing can
  still unlock things
- A slow shot takes time to arrive, an instant weapon still hits on the tick it
  fires, and a ballistic shell misses a target that runs out from under it
- A ground weapon ignores aircraft entirely rather than firing at them
  uselessly, an anti-air gun hits aircraft and cannot touch ground vehicles,
  and a weapon that says nothing about layers is still ground-only

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

---

## Sources

Public documentation of the original's mechanics. Gameplay rules are facts
about how the game behaves; no data files, art or code from any commercial
release are used here or anywhere in this project — see
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md).

- [Spy (Red Alert 2)](https://cnc.fandom.com/wiki/Spy_(Red_Alert_2)) — infiltration effects per building
- [Infiltration](https://cnc.fandom.com/wiki/Infiltration)
- [Tech building](https://cnc.fandom.com/wiki/Tech_building) — the four RA2 tech structures
- [Yuri's Revenge new tech buildings](https://cncnz.com/games/yuris-revenge/new-tech-buildings/) — machine shop, power plant, secret lab
- [Tanya (Red Alert 2)](https://cnc.fandom.com/wiki/Tanya_(Red_Alert_2)) — pistols, C4 targets, swimming, build limit
- [Navy SEAL (Red Alert 2)](https://cnc.fandom.com/wiki/Navy_SEAL_(Red_Alert_2))
- [IFV Weapon System](https://modenc.renegadeprojects.com/IFV_Weapon_System) — 24 turret modes in RA2, 4 more in YR
- [Infantry Fighting Vehicle](https://cnc.fandom.com/wiki/Infantry_Fighting_Vehicle)
- [Engineer (Red Alert 2)](https://cnc.fandom.com/wiki/Engineer_(Red_Alert_2))
- [Disguise Logic](https://modenc.renegadeprojects.com/Disguise_Logic) — what breaks a disguise, and what sees through one
- [Mirage tank (Red Alert 2)](https://cnc.fandom.com/wiki/Mirage_tank_(Red_Alert_2))
- [Allied units](https://cncnz.com/games/red-alert-2/allied-units/) and [Soviet units](https://cncnz.com/games/red-alert-2/soviet-units/)
- [Battle Fortress](https://cnc.fandom.com/wiki/Battle_Fortress), [Prism tank](https://cnc.fandom.com/wiki/Prism_tank_(Red_Alert_2)), [Chrono Miner](https://cnc.fandom.com/wiki/Chrono_Miner)
- [Aircraft carrier (Red Alert 2)](https://cnc.fandom.com/wiki/Aircraft_carrier_(Red_Alert_2)) — the three-Hornet rearm cycle
- [Dreadnought (Red Alert 2)](https://cnc.fandom.com/wiki/Dreadnought_(Red_Alert_2))
- [Iron Curtain (Red Alert 2)](https://cnc.fandom.com/wiki/Iron_Curtain_(Red_Alert_2)) — invulnerability, and that it kills infantry
- [Chronosphere (Red Alert 2)](https://cnc.fandom.com/wiki/Chronosphere_(Red_Alert_2))
- [Crate (Red Alert 2)](https://cnc.fandom.com/wiki/Crate_(Red_Alert_2))

Costs, power figures and prerequisites throughout sections 4 to 7 come from:

- [Allied units](https://cncnz.com/games/red-alert-2/allied-units/)
- [Soviet units](https://cncnz.com/games/red-alert-2/soviet-units/)
- [Soviet structures](https://cncnz.com/games/red-alert-2/soviet-structures/)
- [Allied structures](https://cncnz.com/games/red-alert-2/allied-structures/) — **not yet read**, rate-limited
- [Tech buildings](https://cncnz.com/games/red-alert-2/tech-buildings/) — **not yet read**

Two reimplementations exist and were checked for licence rather than mined for
code: [huangkaoya/redalert2](https://github.com/huangkaoya/redalert2) is
GPL-3.0, and
[ammaarreshi/RedAlert2-Mac-iOS-iPad](https://github.com/ammaarreshi/RedAlert2-Mac-iOS-iPad)
is built on Chrono Divide's **proprietary** engine and is not a usable
reference. Copying from either would make this project a derivative work of
someone else's reimplementation rather than a clean-room one, which is a
different thing from what this project set out to be.
