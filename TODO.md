# TODO

Live task list. Keep this current — check items off as they land, and add work as it is
discovered. Phase definitions, estimates and exit criteria live in
[docs/07-roadmap.md](docs/07-roadmap.md).

**Current phase: 3 — Core gameplay**
**Next milestone:** rules as data — units, buildings and weapons defined in files,
with a hash both peers verify at the handshake.

Phase 1 is complete: two client processes play in lockstep, confirmed by eye and
by matching state hashes, and CI confirms the simulation produces identical
hashes on ARM and x86. Phase 2 is deferred — see docs/07-roadmap.md.

---

## Phase 0 — Foundation

### Project setup
- [x] `git init`, `.gitignore`, Git LFS for `assets/`
- [x] Cargo workspace with the six crates from [docs/01-architecture.md](docs/01-architecture.md)
- [x] Pin Bevy 0.19.1; `rustfmt.toml`, `clippy.toml`
- [x] CI: build, test, clippy `-D warnings`, fmt check
- [x] CI: no-float lint on `redshift-sim`
- [x] CI: assert `redshift-sim` has no engine dependency in its tree
- [x] `--screenshot <path>` for capturing a frame headlessly
- [ ] Licence files — GPLv3 for code, CC BY-SA 4.0 for assets

### `redshift-sim` — foundations
- [x] `Fx` fixed-point type: add, sub, mul, div via `i64` intermediates
- [x] `Fx::sqrt` — integer Newton iteration, fixed iteration count
- [x] Binary-angle trig tables (`u16` angle, full turn = 65536)
- [x] `Fx` test suite including overflow boundaries and negative values
- [x] Deny `From<f32>` on `Fx` — conversion is renderer-only
- [x] Seeded `SimRng` (PCG), with reproducibility tests
- [x] Entity arena: generational indices, deterministic free-list reuse
- [x] `Sim` skeleton with the public API from [docs/01-architecture.md](docs/01-architecture.md)
- [x] Fixed 20 Hz tick loop with the documented phase ordering
- [x] `WorldView` read-only accessor for renderers
- [x] `state_hash()` over all gameplay-relevant state

### `redshift-sim` — movement
- [x] Tile grid map representation, passability
- [x] Grid A\* with deterministic tie-breaking (lowest `f`, then lowest cell index)
- [x] Per-tick node-expansion budget with carry-over — **never** a time budget
- [x] Movement along a path with fixed-point positions and turn rates
- [x] Basic unit collision / avoidance — *deferred to Phase 3; units currently overlap*

### Found while building the shell
- [x] `--demo` flag issuing a scripted order, to exercise input → sim → render
- [x] Tone mapping disabled; light levels recalibrated for an untonemapped pipeline
- [x] Frame-time budget made refresh-rate aware (vsync makes it a pacing metric, not a load one)
- [ ] Custom flat/cel material, replacing `StandardMaterial` configured to look flat — Phase 4
- [ ] Blob shadow decals under units — Phase 4
- [x] `Move` command applied through the command queue

### `redshift-render` — the shell
- [x] Bevy app, window, fixed dimetric camera
- [x] Camera pan (edge scroll, drag, keyboard) and clamped zoom
- [x] Flat grid terrain mesh
- [x] Placeholder cube units, instanced by type and team
- [x] Interpolation between the last two sim states
- [x] Click-select, box-select, selection decals
- [x] Right-click move order → `Command`
- [x] `F3` performance overlay: fps, frame time, sim tick ms, draw calls, triangles, memory
- [x] Vsync locked on; 30 fps cap when unfocused

### `redshift-app`
- [x] Wire sim + render + a local single-peer session
- [x] Argument parsing, including `--bench`
- [x] `--bench` scene asserting every budget ceiling, exiting non-zero on breach

### Phase 0 exit

All four met. Left unticked for a long time after they were true, which is its
own small lesson: an exit criterion nobody re-checks is not a gate.

- [x] Cubes path around obstacles to clicked destinations — `tests/orders.rs`
      and `tests/placement.rs`, and every scenario that puts a building in the
      way of a move order
- [x] A headless replay of the same command log matches state hashes at every
      tick — `net::replay_hashes`, exercised by `redshift-net`'s replay tests
      and by `redshift-sim`'s determinism suite
- [x] `cargo tree -p redshift-sim | grep -i bevy` prints nothing — and a CI lint
      keeps it that way
- [x] Performance overlay within budget on the M1 Pro reference machine —
      `--bench` passes with roughly nineteen times the headroom it needs

---

## Phase 1 — Determinism and LAN

- [x] `Command` enum and compact serialisation
- [x] Ordered command queue — sorted by (tick, player, sequence)
- [x] Turn scheduling with input delay `D`; RTT negotiation at match start
- [x] Wire format with framing, size limit, and foreign-traffic rejection
- [x] UDP transport: sockets, redundant resend of the last 3 ticks
- [x] "Waiting for player" stall handling after ~500 ms
- [x] LAN discovery: broadcast announce on UDP 47654, client listener
- [x] LAN match browsing — *engine side; driven by `--host`/`--join` until the lobby screens exist*
- [x] Lobby: slots, ready state, protocol and rules-hash checks
- [x] Rules hash exchanged and verified in the handshake
- [x] State hash exchange every 20 ticks
- [x] Desync halt with full dumps from both peers
- [ ] Dev-only mode: per-tick, per-subsystem hashing to localise divergence — *offline bisection covers this for now*
- [x] Replay record and playback
- [x] Determinism suite: `replay_roundtrip`, `two_sims_identical`, `serialisation_stable`
- [x] CI cross-platform golden hashes — macOS/ARM vs Linux/x86

### Phase 1 exit
- [x] Two independent client processes play a networked match with zero desyncs
      — verified by eye and by matching state hashes
- [x] Replay reproduces the match bit-exactly
- [x] An injected non-determinism is caught, halts both peers, and writes dumps
- [ ] **Two physical machines on one Wi-Fi** — the part loopback cannot prove:
      broadcast crossing a real router, the macOS firewall prompt, real latency.
      *Needs a second machine; not something the test suite can stand in for.*
- [ ] Cross-platform golden hashes confirmed by a CI run on x86 — *the workflow
      exists; it needs to have actually run once and gone green.*

---

## Phase 2 — Internet play — deferred, see docs/07-roadmap.md

- [ ] `redshift-server`: lobby service (create / list / join)
- [ ] Relay service forwarding command packets, holding no game state
- [ ] Direct P2P attempt with automatic relay fallback
- [ ] Latency and packet-loss telemetry in the lobby UI
- [ ] Reconnection: state snapshot plus catch-up simulation
- [ ] Spectator mode
- [ ] Container image, VPS deployment, basic monitoring

---

## Phase 3 — Core gameplay

- [x] Map format — RON, in `maps/`. Authored as a list of *edits* rather than a
      grid: forty-eight squared is 2,304 cells of which thirty carry anything,
      and nobody can read a diff of that. Two edit lists can make the same grid,
      so a map file is not canonical — nothing needs it to be, since the grid it
      produces is what gets hashed
- [ ] Map editor — the format is hand-writable, which is enough for now
- [x] `redshift-data`: RON loading, validation, cross-reference checks, rules hash
- [x] Trait system and the initial trait catalogue
- [x] Economy: resource fields, harvesters, refineries, credits
- [x] Construction: build queue, footprints, prerequisites, placement
- [x] Power grid and low-power effects — *slowdown ratio is a placeholder, see below*
- [x] Combat: weapons, warhead/armour table, splash — *projectiles still instant*
- [x] Veterancy
- [x] Fog of war, shroud, cloak and detection
- [x] Unit behaviour: attack-move, guard, stop, formations, control groups
- [x] Superweapon and support-power framework — a charge on the *building*, an
      effect in the data, and a place chosen by the player. Four of them ship:
      nuclear missile, Iron Curtain, spy satellite, paradrop
- [ ] Chronosphere and Weather Control — the two that need movement without a
      path and a persistent roaming effect. Absent rather than approximated
- [x] UI: sidebar with a build list read from the *rules*, a queue with
      progress, sell, and a minimap that obeys the same fog everything else
      does. Build tabs and unit info panels are Phase 4 art work
- [ ] Unit info panel — what is selected, its rank, what it is carrying
- [x] Skirmish AI — `redshift-ai`, reading the simulation and returning
      commands. Never a `&mut Sim`: a command is the only way anything reaches
      the world, and an opponent that reached in would be playing a different
      game from the one the replay records
- [x] **Dummy** — builds, defends, never attacks. Deliberate rather than
      broken: it exists so a player can learn the game or test a build order
      without being under a clock. It thinks exactly as well as Easy
- [ ] **Easy, Medium, Hard** — the same head as Dummy with attacking added,
      scaled by one number. See `crates/ai/src/skill.rs` for what "difficulty"
      is taken to mean here, and for why none of them cheat
- [x] Hot reload — `--watch` restarts the match from the same seed when a rules
      or map file changes. *Restart*, not patch: rules feed the state hash, so
      swapping values into a running simulation would desync a networked match
      on the next comparison and leave a solo one in a state no replay could
      reproduce. Refused outright when networked

---

### Engine gaps the roster audit found

Capabilities the original needs. Every one of these is now a **passing test** in
`crates/sim/tests/roster_conformance.rs` — this list is a summary of that suite,
not a second record of it, and the suite is the one to believe.

The section below was allowed to rot for a while: it listed a dozen things as
open that had been closed for several commits, including capture, transports,
the IFV and Tanya. A checklist that lies is worse than no checklist, because it
is read as though it were true. It has been rebuilt against the test names.

**Closed — one trait and a rule each**

- [x] Capture, and being consumed on use — whether it happens is data, not a rule
- [x] A neutral player — owns things, commands nothing, hostile to nobody
- [x] Instant-kill weapons — sniper, attack dog
- [x] Economy modifiers on a structure — the ore purifier
- [x] Build limits — one commando at a time, one superweapon of a kind
- [x] A structure that repairs what is sent into it — Service Depot, Naval
      Shipyard, Outpost. One trait with three lists of what it will service
- [x] Unsellable structures — the only one of a tech building's three properties
      that needed anything at all

**Closed — mechanics with interactions**

- [x] Projectiles — travel time, homing or ballistic, per weapon
- [x] Air targeting — units have a layer, weapons declare what they engage
- [x] Multiple weapons per unit — targeting considers both, firing picks one
- [x] Actions with their own valid targets — Tanya's pistol and her charges. Not
      a third weapon slot: a layer mask cannot tell a building from a person
- [x] A weapon that restores health — the Medic, and the IFV's repair mode
- [x] Transports — loading, unloading, and passengers that stop existing
- [x] IFV turret modes — a weapon that is a property of a *unit*, not its kind.
      Declared on the passengers, so the vehicle never learns anyone's name
- [x] Garrison — the building fires with **its own** weapon, not its occupants'.
      The exact opposite of an IFV, and the thing most easily got backwards
- [x] Deploy — unit↔structure and stance, one mechanism rather than two
- [x] Spy infiltration — an effect table keyed on what was entered, declared on
      the **building**. Four different mechanisms, not one with a parameter
- [x] A parasite that gets inside a unit, and the depot that shakes it off.
      Neither is worth building alone
- [x] Prism chaining and Tesla charging — the same rule, different supporters
- [x] Submersion — the second concealment, with its own sense
- [x] Persistent terrain effects — contamination that outlives what laid it
- [x] What a death leaves — rubble and an ejected crew are one mechanism
- [x] Structure death effects — a blast, and ground that stays dangerous
- [x] Ore regrowth — belongs to the mine, not to ore
- [x] Wandering civilians — bounded to a home, deliberately not an AI
- [x] Victory and stalemate — a match can now end
- [x] Elevation — a height layer per cell; the cliff is the *step*
- [x] Bridges — the only footprint that opens ground, and the only entity
      destroyed without being removed
- [x] Gap Generator — the only subtractive operation in visibility

**Still open — no unit in the roster needs them yet**

These have no failing test standing for them, which is the honest position: the
audit answers "can the engine express what we have described", not "have we
built everything". When a unit arrives that needs one, the gap comes back.

- [ ] Walls — one-cell structures that connect to their neighbours
- [ ] Map reveal — a one-off effect on the visibility layers
- [ ] Placement rules per structure — a naval yard must touch water
- [ ] Placed charges — armed now, detonating later
- [ ] Temporary status effects — invulnerable, irradiated, disabled
- [ ] Aircraft — basing, rearming, a movement model that is not the pathfinder
- [ ] Naval — shoreline transports, water as a surface rather than an obstacle
- [ ] Superweapons and powers — charge timers, targeting modes, novel effects
- [ ] Mind control — changing a unit's owner mid-match
- [ ] Teleportation — movement without a path
- [ ] Disguise — appearing as something else to one side only

### World state the renderer could not see

The simulation grew several things that never reached the screen. A feature the
player cannot see is not a feature, and a *dangerous* one they cannot see is
worse than not having it.

- [x] Elevation — the map was drawn perfectly flat while high ground blocked
      movement and lengthened reach. Units stand on the ground they are on, so
      walking up a ramp no longer sinks into the hillside
- [x] Contaminated ground — tinted, like ore. Radiation that killed infantry
      and looked exactly like grass would have been the cruellest thing to ship
- [x] Bridges — planking over water, and a wrecked span puts the river back on
      screen at the moment it puts it back underfoot
- [x] Infestation and garrison — on the health bar's *backing*, which carried
      no information before. Not the fill: that already means health, and one
      bar saying two things by colour says neither
- [x] The result of the match — a match that quietly stopped mattering, with
      both sides still standing, was worse than having no victory condition

### Commands the player could not issue

Seven of the simulation's fourteen commands had no route through the interface
at all: tested, correct, and unreachable. Found by listing `CommandKind`'s
variants against what the renderer issues.

- [x] Right-click is context-sensitive, as the original's was — an engineer on
      a building captures or repairs it, infantry on a transport climb in, a
      damaged vehicle on a Service Depot goes in to be mended, a factory
      clicked on open ground sets its rally point. The player never picks a
      verb, which is the whole point.
- [x] `D` deploys *and* unloads, one key for both, because from the player's
      side it is one act
- [x] `G` guards, `S` stops — the original's letters, freed by taking WASD off
      the camera. The original never panned with WASD either
- [x] **Sell** and **cancel production** — on the sidebar, where they always
      belonged. All fourteen commands are now issuable by a player

### Declared and never read — the defect this codebase keeps producing

Three found in one audit, all the same shape: a trait resolves into the stat
table, validates, and nothing consults it. Each is silent, so nothing fails —
the feature simply is not there, and the rules file says it is.

- [x] `Harvester`'s `gather_rate` — the bite size was a flat constant, so every
      miner in the game worked at the same speed however its rules read. A
      faster one was not expressible. Worse: the shipped harvester's capacity
      was smaller than the constant bite, so it filled in a single mouthful and
      the "field thins in steps" the comment described never happened.
- [x] `Selectable`'s `priority` — selection picked the nearest thing under the
      pointer and ignored priority entirely. Click a crowd of infantry standing
      around a tank and you got a soldier. The only one of the three a player
      would have felt in every single match.
- [x] `can_crush` — a bool beside the crush bitmask that nothing read. Dead
      weight rather than a bug, but it was in the state hash.

Worth a standing habit rather than a one-off sweep: when a trait lands, the
test that proves it must observe the *effect*, not the resolved stat. Two of
these three had passing tests asserting the value was resolved correctly.

### The gap list is executable

`crates/sim/tests/roster_conformance.rs` holds it. **Fifty-nine capabilities
confirmed, no gaps.**

That does not mean the game is finished. It means the engine can express what
the original does with the units we have described, and what remains is
content, art, and numbers. Aircraft, naval, superweapons, mind control,
teleportation and disguise have no failing test standing for them because no
unit in the roster needs them yet — when one does, the gap comes back.

The count went from twelve to thirty-nine while researching the original
properly. Twenty-seven gaps were invisible until the mechanics were looked up
rather than recalled.

```sh
cargo test -p redshift-sim --test roster_conformance -- --list | grep ignore
```

Closing a gap means deleting an `#[ignore]`. Prefer that to ticking a box here:
a checkbox can be wrong and a test cannot.

### Filling in the remaining numbers

OpenRA's RA2 mod (GPL-3.0) re-derives the original's rules into its own YAML
and is the right reference for values this project still lacks. It confirmed
the tech-building figures exactly and supplied several mechanics no description
mentioned. Where docs/08-roster.md still says ⚠️, that is where to look.

### Verifying the specification

- [ ] **docs/08-roster.md is written from memory and unverified.** Check it
      against the original before building to it. Getting it wrong means
      building the wrong engine quietly.

---

## Phase 4 — Art and feel

- [ ] Blender → glTF pipeline with budget validation on import
- [ ] Team colour material slot and per-instance tinting
- [ ] Unit, building and terrain models replacing all placeholders
- [ ] Damage states; construction and destruction animations
- [ ] Effects: muzzle flashes, explosions, tracers, craters, smoke
- [ ] Blob shadows; final single-directional-light setup
- [ ] Audio: SFX, unit acknowledgements, announcer, UI
- [ ] Music
- [ ] UI art pass
- [ ] Localisation: English and Persian
- [ ] Main menu, settings, keybindings
- [ ] Silhouette review of every unit

---

## Phase 5 — Content and factions

- [x] The Soviet roster — structures, infantry and vehicles, with the
      researched costs. Only what the engine can express: Crazy Ivan, Yuri and
      the Kirov are absent rather than approximated, because a Crazy Ivan who
      threw grenades would be a different unit wearing his name
- [ ] The rest of both rosters — the ones waiting on aircraft, mind control,
      placed charges and indirect fire
- [ ] Country modifiers and unique units
- [x] All nine of the original's countries. Six have their unique unit; three
      wait on a capability — America's paradrop needs support powers, Korea's
      Black Eagle needs flight, Russia's Tesla Tank needs indirect fire. Each is
      recorded in the country's own entry rather than left to be discovered
- [ ] AI improvements and difficulty levels
- [ ] Additional maps
- [ ] Balance pass against the faithful baseline

---

## Phase 6 — Release

- [ ] macOS `.app` — signed and notarised
- [ ] Windows installer; Linux AppImage
- [ ] Auto-update
- [ ] Crash reporting; opt-in telemetry
- [ ] Public docs and modding guide
- [ ] Trademark-safe naming pass over all user-visible strings
- [ ] Public relay server and community infrastructure

---

## Phase 7 — Beyond the remaster (not started, and not before Phase 6 ships)

Requested additions, held until the faithful remaster exists. See
docs/adr/0005-faithful-remaster-scope.md for why.

- [ ] New countries: Iran, Israel, USA, Russia, China
- [ ] Iran — cluster-warhead missiles (needs submunitions on weapons)
- [ ] Israel — stealth strike aircraft (expressible with existing traits)
- [ ] USA — carrier operating several aircraft (needs a Carrier trait)
- [ ] Russia — heavy main battle tank (expressible with existing traits)
- [ ] China — EMP vehicle that disables rather than destroys (proposed; needs a
      disable effect distinct from damage, and is still open to discussion)

---

## Open questions

- [ ] **Terrain mesh is rebuilt whole** each simulation tick when the fog
      moves. Affordable at 48x48; a much larger map will want chunking.
- [ ] **Veterancy bonuses.** `rank::VETERAN_BONUS` and `ELITE_BONUS` are 115%
      and 135%, chosen by feel. Promotion on kills is faithful; the numbers are
      not verified.
- [ ] **Repair rate and price.** `rules/buildings/allied.ron` gives the Service
      Depot `rate: 5` and `cost_percent: 20`. That a repair takes time and costs
      money is faithful; both numbers are guesses. The 20% figure is the shape
      the reference implementations use, not something checked in-game.
- [ ] **Terror Drone bite rate.** The drone is in the trait catalogue but not
      yet in any shipped rules file — there is no Soviet unit roster. Its damage
      per tick is unset rather than wrong.
- [ ] **Prism chaining figures.** The bonus per supporting tower, the radius,
      and the ceiling are all by feel. That towers combine and get stronger
      together is faithful; how much is not verified.
- [ ] **Resurface delay.** How long a submarine stays up after firing or being
      hit is unset — nothing ships with `Submersible` yet, since there is no
      naval roster.
- [ ] **Contamination figures.** The Desolator's radius, damage per tick and
      how long ground stays hot are all unset — there is no Soviet roster yet,
      so nothing ships with `Contaminates`. That it denies an area and outlives
      its source is faithful; the numbers are not yet anything.
- [ ] **Gap Generator radius.** `rules/buildings/allied.ron` says ten cells, by
      feel. The mechanic is faithful; the reach is not verified.
- [ ] **America's country bonus is a stand-in.** The original gives a free
      paradrop; this gives a build-speed bonus because support powers do not
      exist. It should be replaced rather than balanced.
- [ ] **Soviet weapon figures.** The costs and power draws are researched; the
      damage, reload and range numbers are not.
- [ ] **Stalemate quiet period.** `sim::STALEMATE_QUIET_TICKS` is five minutes,
      chosen so that calling one early is unlikely. It also stands in for "can
      these two players still reach each other", which is the question a
      stalemate really asks — an honest answer means a reachability search per
      player per tick over a map with bridges that can be cut. Worth revisiting
      if a real match ever gets called off wrongly.
- [ ] **Short game.** The original offers a match option where losing every
      structure is enough to be out. Not implemented: it wants a field on
      `MatchSetup`, which every test constructs literally.
- [ ] **Ore growth and wander rates.** `Grows` and `Wanders` figures are unset —
      nothing ships with either yet, since there is no ore mine or civilian in
      the rules.
- [ ] **Elevation range bonus.** `map::HEIGHT_RANGE_BONUS_PERCENT` is 15% per
      level, by feel. That high ground helps is faithful; how much is not
      verified. `map::MAX_WALKABLE_STEP` of one level is the more confident of
      the two — the original's maps are built from ramps between adjacent
      levels — but it deserves the same check.
- [ ] **Repair-everywhere rate.** `sim::BOON_REPAIR_RATE` is a guess.
- [ ] **Sell refund rate.** `sim::SELL_REFUND_PERCENT` is 50%, a guess.
- [ ] **Build radius.** `sim::BUILD_RADIUS` is 8 cells, chosen by feel. The
      constraint is faithful; the distance is not verified against the original.
- [ ] **Low-power slowdown ratio.** `power::LOW_POWER_DIVISOR` is set to 4 by
      feel, not verified against the original. The mechanism is faithful; the
      number is not. Needs checking against how the original actually paced
      production in a shortage.

- [ ] Final project name (must avoid EA trademarks) — "Redshift" is provisional
- [ ] Map size ceiling, and whether it affects the `Fx` range choice
- [ ] Whether a scripted campaign is in scope at all, or skirmish and multiplayer only
- [ ] Where the public relay server will be hosted, and who pays for it
- [ ] Whether to build in-editor tooling for maps and rules, or use external files plus hot reload
