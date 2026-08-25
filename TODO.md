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
- [ ] Cubes path around obstacles to clicked destinations
- [ ] A headless replay of the same command log matches state hashes at every tick
- [ ] `cargo tree -p redshift-sim | grep -i bevy` prints nothing
- [ ] Performance overlay within budget on the M1 Pro reference machine

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
      broadcast crossing a real router, the macOS firewall prompt, real latency
- [ ] Cross-platform golden hashes confirmed by a CI run on x86

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

- [ ] Map format: heightmap, tile types, cliffs, buildability — *water, ore and elevation done*
- [ ] Map editor or an authoring-format converter
- [x] `redshift-data`: RON loading, validation, cross-reference checks, rules hash
- [x] Trait system and the initial trait catalogue
- [x] Economy: resource fields, harvesters, refineries, credits
- [x] Construction: build queue, footprints, prerequisites, placement
- [x] Power grid and low-power effects — *slowdown ratio is a placeholder, see below*
- [x] Combat: weapons, warhead/armour table, splash — *projectiles still instant*
- [x] Veterancy
- [x] Fog of war, shroud, cloak and detection
- [x] Unit behaviour: attack-move, guard, stop, formations, control groups
- [ ] Superweapon / support power framework
- [ ] UI: sidebar, build tabs, minimap, unit info — *health bars done*
- [ ] Skirmish AI v1: build order, expansion, attack waves
- [ ] Hot reload of rules in dev builds

---

### Engine gaps the roster audit found

Capabilities the original needs and the engine cannot express. These are engine
work, not content, so they belong in Phase 3 rather than Phase 5. Full detail
and reasoning in docs/08-roster.md.

**Small — a trait and a rule each**

- [ ] Capture — `Capturable` exists and nothing reads it
- [ ] Repair — a structure that heals what comes to it
- [x] Consumed on use — and whether it happens is data, not a rule
- [ ] Walls — one-cell structures that connect to their neighbours
- [ ] Map reveal — a one-off effect on the visibility layers
- [ ] Economy modifiers on a structure
- [x] A neutral player — owns things, commands nothing, hostile to nobody
- [ ] Instant-kill weapons — sniper, attack dog
- [ ] Placement rules per structure — a naval yard must touch water

**Medium — real mechanics with interactions**

- [x] **Projectiles** — travel time, homing or ballistic, per weapon
- [x] **Air targeting** — units have a layer, weapons declare what they engage
- [x] A weapon that restores health — the Medic, and the IFV's repair mode.
      Targeting inverts; it is not a damage number with a minus sign.
- [x] Ore regrowth — belongs to the mine, not to ore, which keeps the contrast
      between a field worth holding and one worth stripping.
- [x] What a death leaves — rubble and an ejected crew are one mechanism.
- [x] Structure death effects — a blast, and ground that stays dangerous.
- [x] Tesla charging — the same rule as Prism chaining with a different list.
- [x] Wandering civilians — bounded to a home, deliberately not an AI.
- [x] Tech structures — two of the three properties needed nothing at all.
- [x] Victory and stalemate — a match can now end.
- [x] Actions with their own valid targets — Tanya's pistol and her charges.
- [x] IFV turret modes — a weapon that is a property of a *unit* rather than of
      its kind. Declared on the passengers, so a new unit brings its own mode
      and the vehicle never learns anyone's name.
- [x] Prism chaining — the first stat that depends on the neighbours. Rebuilt
      each tick like the power grid.
- [x] Submersion — the second concealment, with its own sense. Being hit
      surfaces a submarine, not only firing; and "only sonar can engage one"
      falls out of the concealment rather than being a targeting rule.
- [x] Persistent terrain effects — ground the Desolator poisons, which outlives
      whatever laid it. No "immune to radiation" flag: the armour table already
      answers that, so one row makes infantry die on ground a tank drives over.
- [x] Bridges — the only footprint that *opens* ground rather than claiming it,
      and the only entity destroyed without being removed. Repaired through a
      hut beside them, which makes it capture with a different effect rather
      than a new mechanic.
- [x] Gap Generator — the only subtractive operation in visibility. Vision runs
      in three passes; the order is the design.
- [x] Spy infiltration — an effect table keyed on what was entered, declared on
      the **building** rather than on the spy. All five rows: promotion by
      category, a timed blackout, theft of a share of the funds, and stolen
      technology. They are four different mechanisms, not one with a parameter.
- [x] Garrison — infantry occupying a civilian building and fighting from
      inside it. The building fires with **its own** weapon, not its occupants'
      — the exact opposite of an IFV, and the thing most easily got backwards.
      Only a neutral building can be occupied and an emptied one reverts, which
      is both faithful and why nothing has to remember who owned it first.
- [x] A structure that repairs what is sent into it — Service Depot, Naval
      Shipyard, Outpost. One trait with three lists of what it will service,
      billed on a running total so the price is exact whatever the step size.
- [x] A parasite that gets inside a unit — the Terror Drone, and the depot that
      shakes it off. Neither is worth building alone: a drone with no counter
      is a death sentence, and a repair shed with nothing to undo is furniture.
- [x] Deploy — unit↔structure, and stance toggling. One mechanism, not two:
      the deployed form is an ordinary entity whose own `Deploys` points back,
      so undeploying is deploying in the other direction. `G` in the client.
      Brought the Service Depot in with it, since the MCV needs it in the tech
      tree.
- [ ] Transports — loading, unloading, passengers that fire or change the weapon
- [x] Multiple weapons per unit — targeting considers both, firing picks one
- [ ] **A unit chooses between actions by what it is aimed at** — Tanya shoots
      units and demolishes buildings; a tank fires and crushes. Capability is a
      list, not a slot
- [ ] Placed charges — armed now, detonating later
- [ ] Temporary status effects — invulnerable, irradiated, disabled
- [ ] Tech structures — neutral, capturable, unsellable, extend the build radius
- [x] Persistent production modifiers — one mechanism, three effects
- [x] Instant-kill weapons
- [ ] A unit's weapon depending on its cargo — the IFV's 24 turret modes
- [ ] Build limits — only one commando at a time, two with a cloning vat
- [ ] Wandering civilians — autonomous, purposeless movement

### Subsystems needing a decision before a schedule

- [x] **Elevation** — a height layer per cell. The cliff is the *step* between
      levels, not the plateau, so high ground is standable; one level is a ramp
      and two is a wall. Standing higher lengthens sight and weapon range
      together, so a unit never shoots into fog or spots what it cannot hit.
      Probably not deferrable: it changes the map format everything is built on
- [ ] Aircraft — basing, rearming, a movement model that is not the pathfinder
- [ ] Naval — shoreline transports, water as a surface rather than an obstacle
- [ ] Superweapons and powers — charge timers, targeting modes, novel effects
- [ ] Mind control — changing a unit's owner mid-match
- [ ] A weapon that restores health — the Medic, Yuri's repair drones, and the
      one IFV turret mode the mechanism cannot express. It wants targeting to
      invert for such a weapon (friendly and damaged rather than hostile),
      which is a change to what a target *is* rather than a damage number with
      a minus sign.
- [ ] Teleportation — movement without a path
- [ ] Disguise — appearing as something else to one side only

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

- [ ] Complete rosters for both sides
- [ ] Country modifiers and unique units
- [ ] All nine of the original's countries, each with its unique unit and modifier
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
