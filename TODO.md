# TODO

Live task list. Keep this current — check items off as they land, and add work as it is
discovered. Phase definitions, estimates and exit criteria live in
[docs/07-roadmap.md](docs/07-roadmap.md).

**Current phase: 0 — Foundation**
**Next milestone:** cubes moving on a grid, on a deterministic simulation, within budget.

---

## Phase 0 — Foundation

### Project setup
- [ ] `git init`, `.gitignore`, Git LFS for `assets/`
- [ ] Cargo workspace with the six crates from [docs/01-architecture.md](docs/01-architecture.md)
- [ ] Pin Bevy 0.19.1; `rustfmt.toml`, `clippy.toml`
- [ ] CI: build, test, clippy `-D warnings`, fmt check
- [ ] CI: no-float lint on `redshift-sim`
- [ ] CI: assert `redshift-sim` has no engine dependency in its tree
- [ ] Licence files — GPLv3 for code, CC BY-SA 4.0 for assets

### `redshift-sim` — foundations
- [ ] `Fx` fixed-point type: add, sub, mul, div via `i64` intermediates
- [ ] `Fx::sqrt` — integer Newton iteration, fixed iteration count
- [ ] Binary-angle trig tables (`u16` angle, full turn = 65536)
- [ ] `Fx` test suite including overflow boundaries and negative values
- [ ] Deny `From<f32>` on `Fx` — conversion is renderer-only
- [ ] Seeded `SimRng` (PCG), with reproducibility tests
- [ ] Entity arena: generational indices, deterministic free-list reuse
- [ ] `Sim` skeleton with the public API from [docs/01-architecture.md](docs/01-architecture.md)
- [ ] Fixed 20 Hz tick loop with the documented phase ordering
- [ ] `WorldView` read-only accessor for renderers
- [ ] `state_hash()` over all gameplay-relevant state

### `redshift-sim` — movement
- [ ] Tile grid map representation, passability
- [ ] Grid A\* with deterministic tie-breaking (lowest `f`, then lowest cell index)
- [ ] Per-tick node-expansion budget with carry-over — **never** a time budget
- [ ] Movement along a path with fixed-point positions and turn rates
- [ ] Basic unit collision / avoidance
- [ ] `Move` command applied through the command queue

### `redshift-render` — the shell
- [ ] Bevy app, window, fixed dimetric camera
- [ ] Camera pan (edge scroll, drag, keyboard) and clamped zoom
- [ ] Flat grid terrain mesh
- [ ] Placeholder cube units, instanced by type and team
- [ ] Interpolation between the last two sim states
- [ ] Click-select, box-select, selection decals
- [ ] Right-click move order → `Command`
- [ ] `F3` performance overlay: fps, frame time, sim tick ms, draw calls, triangles, memory
- [ ] Vsync locked on; 30 fps cap when unfocused

### `redshift-app`
- [ ] Wire sim + render + a local single-peer session
- [ ] Argument parsing, including `--bench`
- [ ] `--bench` scene asserting every budget ceiling, exiting non-zero on breach

### Phase 0 exit
- [ ] Cubes path around obstacles to clicked destinations
- [ ] A headless replay of the same command log matches state hashes at every tick
- [ ] `cargo tree -p redshift-sim | grep -i bevy` prints nothing
- [ ] Performance overlay within budget on the M1 Pro reference machine

---

## Phase 1 — Determinism and LAN

- [ ] `Command` enum and compact serialisation
- [ ] Ordered command queue — sorted by (tick, player, sequence)
- [ ] Turn scheduling with input delay `D`; RTT negotiation at match start
- [ ] UDP transport: sequence numbers, redundant resend of the last 3 ticks
- [ ] "Waiting for player" stall handling after ~500 ms
- [ ] LAN discovery: broadcast announce on UDP 47654, client listener
- [ ] In-game LAN match list
- [ ] Lobby: slots, colours, ready state, protocol version check
- [ ] Rules hash exchanged and verified in the handshake
- [ ] State hash exchange every 20 ticks
- [ ] Desync halt with full dumps from both peers
- [ ] Dev-only mode: per-tick, per-subsystem hashing to localise divergence
- [ ] Replay record and playback
- [ ] Determinism suite: `replay_roundtrip`, `two_sims_identical`, `serialisation_stable`
- [ ] CI cross-platform golden hashes — macOS/ARM vs Linux/x86

### Phase 1 exit
- [ ] Two machines, 10-minute match, hundreds of units, zero desyncs
- [ ] Replay reproduces the match bit-exactly
- [ ] An injected non-determinism is caught within one second of sim time

---

## Phase 2 — Internet play

- [ ] `redshift-server`: lobby service (create / list / join)
- [ ] Relay service forwarding command packets, holding no game state
- [ ] Direct P2P attempt with automatic relay fallback
- [ ] Latency and packet-loss telemetry in the lobby UI
- [ ] Reconnection: state snapshot plus catch-up simulation
- [ ] Spectator mode
- [ ] Container image, VPS deployment, basic monitoring

---

## Phase 3 — Core gameplay

- [ ] Map format: heightmap, tile types, cliffs, water, buildability
- [ ] Map editor or an authoring-format converter
- [ ] `redshift-data`: RON loading, validation, cross-reference checks, rules hash
- [ ] Trait system and the initial trait catalogue
- [ ] Economy: resource fields, harvesters, refineries, credits
- [ ] Construction: build queue, placement rules, prerequisites
- [ ] Power grid and low-power effects
- [ ] Combat: weapons, projectiles, warhead/armour table, splash
- [ ] Veterancy
- [ ] Fog of war, shroud, vision and detector traits
- [ ] Unit behaviour: attack-move, guard, stop, formations, control groups
- [ ] Superweapon / support power framework
- [ ] UI: sidebar, build tabs, minimap, unit info, health bars
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
- [ ] **The new country** — added with zero Rust changes
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

## Open questions

- [ ] Final project name (must avoid EA trademarks) — "Redshift" is provisional
- [ ] Map size ceiling, and whether it affects the `Fx` range choice
- [ ] Whether a scripted campaign is in scope at all, or skirmish and multiplayer only
- [ ] Where the public relay server will be hosted, and who pays for it
- [ ] Whether to build in-editor tooling for maps and rules, or use external files plus hot reload
