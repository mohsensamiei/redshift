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

Sections 2, 2b and 4b were **researched and verified** against public
documentation of the original; sources are listed at the end. The rest is
**still written from memory and unverified**, and is marked ⚠️ where it is a
guess.

That split is deliberate rather than laziness: the verified parts are the ones
the engine turned out to be furthest from, so they were worth the time first.
The remainder still needs the same treatment.

**The parts written from memory are not yet safe to build against.** Names, numbers and specific behaviours
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

**As of the last run: 16 capabilities confirmed, 18 gaps.**

The gap count went *up* after research, which is the point of doing it: six of
those eighteen were invisible until the mechanics were actually looked up.

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

Two reimplementations exist and were checked for licence rather than mined for
code: [huangkaoya/redalert2](https://github.com/huangkaoya/redalert2) is
GPL-3.0, and
[ammaarreshi/RedAlert2-Mac-iOS-iPad](https://github.com/ammaarreshi/RedAlert2-Mac-iOS-iPad)
is built on Chrono Divide's **proprietary** engine and is not a usable
reference. Copying from either would make this project a derivative work of
someone else's reimplementation rather than a clean-room one, which is a
different thing from what this project set out to be.
