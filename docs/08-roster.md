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

Every section is now researched. What remains unverified is marked ⚠️ inline —
mostly exact figures the sources did not state, such as how far apart Prism
Towers may be and still chain, or how many infantry a given building holds.

Names are the original's, used to identify what is being described. They are
not the names that ship — see
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md). Names, numbers and specific behaviours
need checking against the original before any of it is treated as settled. It is
accurate enough to audit the engine against, which is what it is for. Entries
that are guesses are marked ⚠️.

---

## 1. Terrain and map features

Researched.

| Feature | Behaviour | Engine |
|---|---|---|
| Ground | Buildable, drivable | ✅ |
| Water | Naval only, unless amphibious | ✅ |
| Cliffs / elevation | Blocks ground movement, and **units on high ground have the advantage** — greater effective range | ✅ a height layer per cell; the cliff is the *step*, and the plateau is standable |
| Ramps | The only way between elevations | ✅ a one-level step is walkable, two is a cliff face |
| Shore | Where amphibious transports load and unload | ❌ |
| Bridges | Crossable, **destructible** — Crazy Ivan is the usual way — and **repaired by an engineer entering a separate repair hut beside them** | ❌ |
| Ore | Gathered by faction-specific miners. **Can be destroyed** by force-firing on it with a weapon allowed to | ⚠️ ore ✅, destroying it ❌ |
| Gems | Worth more per load than ore | ❌ second resource kind |
| Trees, rocks | Block movement | ⚠️ blocking only |

Two corrections to what this document previously guessed.

**Bridges are repaired through a hut, not by touching the bridge.** The hut is
a separate capturable-style structure beside the bridge, which makes bridge
repair the same mechanic as capturing a tech building rather than a new one.

**High ground gives a range advantage**, not merely a movement restriction.
Modelling elevation as impassable rock — which is what Redshift used to do —
loses the part that actually affects a fight. It is now a height layer parallel
to terrain and ore: the cliff is the *step* between levels rather than the
plateau, so high ground is somewhere a unit stands and fights, and standing
there lengthens both its sight and its reach. The size of that bonus
(`HEIGHT_RANGE_BONUS_PERCENT`, 15% per level) is a guess and is flagged with the
project's other unverified rates.

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

Researched, and more specific than "infantry can garrison buildings":

- **Only basic infantry** garrison. A GI or a Conscript can; a commando cannot.
- **Capacity depends on the building's size**, so it is a property of the
  building rather than a constant.
- The occupied building **fires with its own predetermined weapon**, one for
  each side — *not* the weapon of whoever is inside. This is the opposite of
  how the IFV works, and worth not confusing.
- The garrison can be **ordered out**, and is **forced out below 33% health**.

That last rule is the interesting one: a garrisoned building is not a death
trap, and clearing one means damaging it enough to evict rather than destroying
it outright.

### Tech structures — captured by an engineer

Verified. These are neutral buildings scattered on maps, marked with a yellow
flag, captured by walking an engineer in.

| Structure | Effect | Game |
|---|---|---|
| **Oil Derrick** | **$1000 immediately, then $20 per second** for as long as it is held | RA2 |
| **Hospital** | Heals infantry the owner **orders to enter it** | RA2 |
| **Airport** | Grants the **paradrop** support power | RA2 |
| **Outpost** | **Repairs vehicles ordered into it**, and is armed with a modified Patriot launcher that hits **ground and air** | RA2 |
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
| Allied | America | **Airborne** — a paradrop power, not a unit |
| Allied | Great Britain | Sniper |
| Allied | France | Grand Cannon |
| Allied | Germany | Tank Destroyer |
| Allied | Korea | Black Eagle |
| Soviet | Russia | Tesla Tank |
| Soviet | Cuba | Terrorist |
| Soviet | Iraq | Desolator |
| Soviet | Libya | Demolition Truck |

Confirmed against the source. Nine, and exactly these.

The pattern is one unique unit or power each, on a shared side roster. The data
layer already expresses `unique_units`, `removes_units` and `modifiers`; none of
it has been exercised.

**Two of these are not units at all.** Paratroopers are a power with a cooldown,
and ground denial is a persistent area effect. Both need mechanics that do not
exist.

---

## 4. Structures and the tech tree

Both sides researched. They mirror each other closely, and the differences are
the interesting part.

| Structure | Cost | Power | Needs | Notes | Engine |
|---|---|---|---|---|---|
| Construction Yard | 3000 | 0 | — | Built by deploying an MCV | ✅ |
| Tesla Reactor | 600 | **+150** | — | | ✅ |
| Ore Refinery | 2000 | −50 | Reactor | **Comes with a free miner** | ❌ a building that spawns a unit |
| Barracks | 500 | −10 | Reactor | | ✅ |
| War Factory | 2000 | −25 | Refinery, Barracks | | ✅ |
| Naval Shipyard | 1000 | −20 | Refinery | **Must be placed in water**; ships are **repaired** here | ❌ placement rule, ❌ repair |
| Radar Tower | 1000 | −50 | Refinery | **Stops working when power is short** | ⚠️ low power slows production; it does not disable |
| Service Depot | 800 | −20 | War Factory | Repairs vehicles; **removes a Terror Drone** | ⚠️ exists in the tech tree, because the MCV needs it; repairing is still a gap |
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

### Where the Allied side differs

| Structure | Cost | Power | Needs | Notes |
|---|---|---|---|---|
| Power Plant | 600 | **+200** | — | More than a Tesla Reactor's +150 |
| Airforce Command HQ | 1000 | −50 | Refinery | Radar **and four aircraft pads** — the Soviets have no equivalent |
| Ore Purifier | 2500 | −200 | Refinery, Lab | **+25% credits from every load**. One per player |
| Pillbox | 500 | **0** | Barracks | Anti-infantry, needs no power |
| Patriot Missile | 1000 | −50 | Barracks | Anti-air; **intercepts missiles** |
| Prism Tower | 1500 | −75 | Air HQ | **Combines beams with nearby towers**, the more chained the stronger |
| Gap Generator | 1000 | −100 | Battle Lab | **Hides the base from enemy radar** — imposing fog on someone else |
| Spy Satellite Uplink | 1000 | −100 | Battle Lab | Reveals the whole map |
| Chronosphere | 2500 | −200 | Battle Lab | One per player |
| Weather Control | 5000 | −200 | Battle Lab | One per player |

Three mechanics here that exist on neither earlier list:

- **A structure that boosts other structures of its own kind.** Prism Towers
  chain, and the beam gets stronger with each tower in the chain. Nothing in
  the engine lets one entity's stats depend on its neighbours.
- **Imposing fog on an opponent.** The Gap Generator does not reveal ground for
  its owner; it *hides* ground from everyone else. Redshift's visibility is
  purely additive.
- **An economy multiplier.** The Ore Purifier changes the value of every load
  delivered, which is a standing modifier on a player rather than on a unit.

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

**Units that deploy** are a category rather than an exception — and, it turns
out, not two mechanisms but one. An MCV becoming a Construction Yard is plainly
a transformation; a GI "changing stance" is also one, once you accept that
something which cannot move and shoots differently is not the same unit with a
flag set. So the deployed form is an ordinary entity whose own `Deploys` points
back, and undeploying is deploying in the other direction. Every row below
comes out of that one trait:

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
| GI | A | 200 | — | **Deploys** into a machine-gun emplacement: more range and power, cannot move. **Can garrison** civilian buildings | ✅ deploy, ❌ garrison |
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
| Desolator | Iraq | 600 | Radar | Melts infantry; **deployed, irradiates ground and makes it impassable** | ✅ deploy, ❌ terrain-altering effect |
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
| MCV | both | 3000 | Service Depot | **Becomes a Construction Yard**, and back | ✅ |
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

## 8b. Reference implementations, and what each is good for

Four were checked. The conclusion differs sharply between them, and the
distinction is worth recording because it determines what can be used.

| Repository | Licence | Useful for |
|---|---|---|
| [OpenRA/ra2](https://github.com/OpenRA/ra2) | **GPL-3.0** | **Actual values.** Re-derived rules in YAML, not the original's files |
| [huangkaoya/redalert2](https://github.com/huangkaoya/redalert2) | GPL-3.0 | The *list* of traits a working engine needed |
| [chronodivide/mod-sdk](https://github.com/chronodivide/mod-sdk) | none declared | Documentation of a rules format |
| [ammaarreshi/…](https://github.com/ammaarreshi/RedAlert2-Mac-iOS-iPad) | proprietary engine | **Nothing.** Not usable as a reference |

### The one that matters

**OpenRA's RA2 mod is the right reference for numbers.** It is GPL-3.0 — the
same licence as this project — and, crucially, its rules are **re-derived into
its own YAML format** rather than parsed from the original's data files. That
is the same thing this project set out to do, which makes it a legitimate
cross-check rather than a shortcut around
[adr/0004-original-assets-only.md](adr/0004-original-assets-only.md).

It confirmed the tech-building research exactly — an oil derrick pays $1000 on
capture and trickles $20 on an interval — and supplied things no wiki stated:

- A **civilian has 50 health and costs 10**, and **killing one pays $5**.
  Civilians are not only scenery; they are a (tiny) income source, which is a
  reason to shoot them beyond spite.
- A tech oil derrick is **2×2**, has 1000 health, **explodes when destroyed**,
  and **leaves rubble behind** as a separate entity.
- Civilians are **mind-controllable**, which follows from being infantry but is
  not something any description mentions.

Rubble is a mechanic nothing else surfaced: a destroyed building leaves an
object behind rather than clearing the ground.

### What the others gave

`huangkaoya/redalert2` **cannot supply values** — its entire data layer is file
format parsers (MIX, SHP, VXL, INI) and it reads the player's own copy at
runtime. What it could give is a **table of contents**: the names of the traits
a shipped reimplementation needed. Reading that list is ordinary research;
copying code would make this a derivative of someone else's reimplementation
rather than the clean-room one it set out to be.

That cross-check found **eight mechanics this document had missed entirely**:

The repositories mentioned at the start of this project were checked. The
finding is worth recording because it is not what was expected:

| Mechanic | Why it matters |
|---|---|
| **Ammunition and reloading** | Units carry finite shots and must return to rearm. Assumed to be an aircraft-only rule; it is a general one |
| **Crew ejection** | A destroyed vehicle releases surviving infantry. Changes the value of every vehicle kill |
| **Rally points** | Where newly built units go. Redshift drops them beside the factory and stops |
| **Selling structures** | A refund, and for the Cloning Vats a way of converting units to cash |
| **Ore regrowth** | Ore spreads from a source over time, so a field is renewable rather than finite |
| **Radiation as map state** | The Desolator leaves ground contaminated. Terrain that damages what stands on it |
| **Idle actions** | The aimless animations that make civilians read as alive |
| **Stalemate detection** | The match has to be able to decide nobody can win |

### Two more from OpenRA's data

| Mechanic | Why it matters |
|---|---|
| **Rubble** | A destroyed building leaves an object behind rather than clearing its ground |
| **Bounty on kills** | Killing a civilian pays a few credits. Every unit may carry a payout |

Ore regrowth is the one with the widest reach: Redshift's economy assumes a
fixed quantity of ore on the map, and a renewable one changes how long a match
runs and whether a contested field is worth holding.

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

**As of the last run: 39 capabilities confirmed, 18 gaps.**

The gap count went *up* after research, twice, which is the point of doing it.
Twenty-eight of those forty were invisible until the mechanics were researched
rather than recalled. One has since been closed — the power model.

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

Grouped by how much each represents. **Unnumbered on purpose**: this list used
to be numbered, and closing items left holes in the sequence that the prose
around it then referred to wrongly — the same drift that section 9 exists to
stop. The count and the reasons live in the audit; this is only the shape.

Items are struck through as they close, so the history stays readable:

### Small — a trait and a rule each

- ~~Capture~~ — `Capturable` exists, unread
- ~~Repair~~ — a structure that heals what comes to it
- ~~Consumed on use~~ — the engineer disappearing
- ~~Walls~~ — one-cell structures that connect
- ~~Map reveal~~ — a one-off effect on the visibility layers
- ~~Economy modifiers on a structure~~
- ~~A neutral player~~ — owns things, commands nothing, hostile to nobody
- ~~Instant-kill weapons~~ — sniper, attack dog
- **Placement rules per structure** — must touch water
- ~~Deploy~~, both kinds: unit↔structure, and stance toggling. They turned out
  to be one mechanism, not two

### Medium — real mechanics with interactions

- **Garrison** — passengers firing from a building, evicted when it falls
- ~~Transports~~ — loading, unloading, passengers that fire
- **A passenger that changes its carrier's weapon** — the IFV, still open
- ~~Projectiles~~ — shots used to land instantly
- ~~Air targeting~~ — nothing used to distinguish an air target from a ground one
- ~~Multiple weapons per unit~~ — anti-ground and anti-air on one chassis
- **Placed charges** — armed now, detonating later
- **Temporary status effects** — invulnerable, irradiated, disabled
- **Wandering civilians** — autonomous, purposeless movement

### Large — each is a subsystem

- ~~Elevation~~ — real height, ramps, and its effect on sight and combat
- **Aircraft** — basing, rearming, a separate movement model
- **Naval** — shoreline transports, water as a surface
- **Superweapons and powers** — timers, targeting modes, novel effects
- **Mind control** — changing a unit's owner mid-match
- **Teleportation** — movement without a path
- **Disguise** — appearing as something else to one side only
- **Bridges** — destructible terrain that changes connectivity

## 11. What follows

**The Phase 3 exit criteria are wrong.** They say a 1v1 skirmish is playable,
which is nearly true, and say nothing about the roster being expressible. The
small and medium items above are engine capability rather than content, which
makes them Phase 3 work by any reasonable reading, and none of them were in the
plan.

**The two foundational ones are done.** Shots used to land instantly and nothing
distinguished an air target from a ground one; everything built on the combat
model would have had to be revisited when those changed, which is why they went
first. Elevation went next for the same reason — it changes the map format that
everything else is built on.

**The large items need a decision, not a schedule.** Some may be Phase 5 work
done alongside the units that need them.

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
- [Allied structures](https://cncnz.com/games/red-alert-2/allied-structures/)
- [Tech buildings](https://cncnz.com/games/red-alert-2/tech-buildings/)
- [Garrisoning](https://cnc.fandom.com/wiki/Garrisoning) — capacity, the building's own weapon, eviction below a third health
- [Prism tower (Red Alert 2)](https://cnc.fandom.com/wiki/Prism_tower_(Red_Alert_2)) — beam chaining
- [Ore](https://cnc.fandom.com/wiki/Ore) — that ore can be destroyed by force-fire
- [Tech buildings](https://cncnz.com/games/red-alert-2/tech-buildings/) — exact figures: $1000 and $20/sec
- [Factions](https://cncnz.com/games/red-alert-2/factions/) — the nine countries, confirmed

Values cross-checked against [OpenRA/ra2](https://github.com/OpenRA/ra2)
(GPL-3.0), whose rules are re-derived into its own format rather than read from
the original's data files — the same approach this project takes, which is what
makes it a legitimate reference rather than a way around ADR 0004.

Two reimplementations exist and were checked for licence rather than mined for
code: [huangkaoya/redalert2](https://github.com/huangkaoya/redalert2) is
GPL-3.0, and
[ammaarreshi/RedAlert2-Mac-iOS-iPad](https://github.com/ammaarreshi/RedAlert2-Mac-iOS-iPad)
is built on Chrono Divide's **proprietary** engine and is not a usable
reference. Copying from either would make this project a derivative work of
someone else's reimplementation rather than a clean-room one, which is a
different thing from what this project set out to be.
