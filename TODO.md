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

- [ ] Map format: heightmap, tile types, cliffs, buildability — *water and ore done*
- [ ] Map editor or an authoring-format converter
- [ ] `redshift-data`: RON loading, validation, cross-reference checks, rules hash
- [ ] Trait system and the initial trait catalogue
- [x] Economy: resource fields, harvesters, refineries, credits
- [x] Construction: build queue, footprints, prerequisites, placement
- [x] Power grid and low-power effects — *slowdown ratio is a placeholder, see below*
- [x] Combat: weapons, warhead/armour table, splash — *projectiles still instant*
- [ ] Veterancy
- [x] Fog of war and shroud — *Detector and Cloakable traits still unread*
- [ ] Unit behaviour: attack-move, guard, stop, formations, control groups
- [ ] Superweapon / support power framework
- [ ] UI: sidebar, build tabs, minimap, unit info — *health bars done*
- [ ] Skirmish AI v1: build order, expansion, attack waves
- [ ] Hot reload of rules in dev builds

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
