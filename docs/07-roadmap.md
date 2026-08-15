# 07 — Roadmap

## Ordering principle

**Prove the risky things first, not the fun things.**

The risk in this project is not art. Art is expensive but predictable — you know at the outset
roughly how long a model takes, and no amount of art can fail catastrophically. The risk is
**deterministic simulation and netcode**: it either works or the whole multiplayer premise
collapses, and the failure surfaces late and is hard to diagnose.

So: cubes on a grid get networked *before* a single tank is modelled. Spending a week on a
beautiful tank before knowing whether two machines can stay in sync is the classic way these
projects die.

Time estimates assume one developer working with AI assistance, part-time. They are planning
aids, not commitments.

---

## Phase 0 — Foundation

*Goal: a window, a grid, and cubes that move on command — running on a deterministic simulation.*

- Cargo workspace, CI, clippy/rustfmt config, the no-float lint
- `Fx` fixed-point type: arithmetic, `sqrt`, trig tables, full test coverage
- Seeded deterministic RNG
- Entity arena with generational indices
- Fixed 20 Hz tick loop, headless-capable
- Grid A\* pathfinding with deterministic tie-breaking and a per-tick node budget
- Bevy app shell: window, fixed dimetric camera, pan and clamped zoom
- Flat grid terrain, placeholder cube units
- Click-select, box-select, right-click move order
- Render interpolation between sim ticks
- `F3` performance overlay: fps, frame time, sim tick time, draw calls, triangles, memory

**Exit criteria**
- Cubes move to clicked destinations, pathing around obstacles
- The same command log replayed headless produces identical state hashes at every tick
- `cargo tree -p redshift-sim | grep -i bevy` returns nothing
- The performance overlay is within budget on the reference machine

*Estimate: 3–5 weeks*

---

## Phase 1 — Determinism and LAN

*Goal: two laptops on the same Wi-Fi play the cube game with zero desyncs.*

- Command type and serialisation
- Turn scheduling with negotiated input delay
- UDP transport with sequence numbers and redundant recent-tick resend
- LAN discovery via UDP broadcast; in-game match list
- Lobby: player slots, colours, ready state, protocol version check
- `state_hash()` over all sim state
- Hash exchange every 20 ticks; desync halt with dumps from both peers
- Replay recording and playback
- Determinism test suite, including cross-platform golden hashes in CI (ARM + x86)

**Exit criteria**
- Two machines run a 10-minute match with hundreds of units and zero desyncs
- A recorded replay reproduces the match bit-exactly
- A deliberately injected non-determinism is caught by CI within one second of sim time

*This is the phase that decides whether the project is viable. Do not skip its tests.*

*Estimate: 4–6 weeks*

---

## Phase 2 — Internet play

*Goal: two players on different networks play, with no port forwarding.*

- `redshift-server`: lobby service (create/list/join matches)
- Relay service: forward command packets between a match's peers
- Direct P2P attempt with automatic relay fallback
- Latency and packet-loss telemetry surfaced in the lobby
- Reconnection: snapshot plus catch-up
- Spectator mode (receive and simulate, send nothing)
- Deployment: container image, single small VPS, basic monitoring

**Exit criteria**
- Two players on different ISPs complete a match through the relay
- A player who disconnects mid-match rejoins and resyncs
- The server sustains 20 concurrent matches within a small VPS's resources

*Estimate: 3–4 weeks*

---

## Phase 3 — Core gameplay

*Goal: a genuinely playable 1v1 skirmish — with placeholder art throughout.*

- Map format, terrain with height levels, cliffs, water, buildable/unbuildable surfaces
- Basic map editor (or a converter from a simple authored format)
- Economy: resource fields, harvesters, refineries, credits
- Construction: build queue, placement rules, prerequisites, power grid and low-power effects
- Combat: weapons, projectiles, warhead/armour damage table, splash, veterancy
- Fog of war and shroud, with vision and detector traits
- Unit behaviour: attack-move, guard, stop, formations, group hotkeys
- Superweapons and support powers framework
- UI: right sidebar, build tabs, minimap, unit info, health bars, control groups
- Skirmish AI, initial version (build order, expansion, attack waves)

**Exit criteria**
- A complete 1v1 skirmish is playable start to finish, human vs human and human vs AI
- Everything still runs in lockstep with no desyncs
- Still within the performance budget with 400+ units on the field

*Estimate: 3–5 months — the largest phase*

---

## Phase 4 — Art and feel

*Goal: it stops looking like a prototype and starts looking like the game you remember.*

- Asset pipeline: Blender → glTF → runtime, with import validation against budgets
- Unit, building and terrain models replacing all placeholders
- Team colour, damage states, construction and destruction animations
- Effects: muzzle flashes, explosions, tracers, craters, smoke — as quads and decals
- Blob shadows and the single-directional-light setup finalised
- Audio: SFX, unit acknowledgements, announcer, UI sounds
- UI art passes to match the original's visual language
- Localisation: English and Persian
- Main menu, settings, keybindings

**Exit criteria**
- No placeholder art remains in a standard skirmish
- Silhouette test: every unit identifiable as a black shape at default zoom
- Performance budget still met; fan still off during a large battle

*Estimate: 4–8 months, dominated by art volume*

---

## Phase 5 — Content and factions

*Goal: full rosters, plus the new country.*

- Complete unit and structure rosters for both sides
- Country modifiers and unique units for the original countries
- **The new country**, added purely through data and art
- Skirmish AI improvements, difficulty levels
- More maps
- Balance pass against the faithful baseline

**Exit criteria**
- The new country is added with **zero Rust changes** — one RON file, one model, one voice set
- AI plays a competent game on all difficulties
- Multiplayer balance is stable across a run of test matches

*Estimate: 2–4 months*

---

## Phase 6 — Release

- Packaging: signed and notarised macOS `.app`, Windows installer, Linux AppImage
- Auto-update
- Crash reporting; opt-in telemetry
- Public documentation, modding guide
- Trademark-safe naming pass across all user-visible strings
- Community infrastructure: issue templates, contribution guide, public server

*Estimate: 1–2 months*

---

## Summary

| Phase | Deliverable | Estimate |
|---|---|---|
| 0 | Foundation — cubes moving on a deterministic sim | 3–5 weeks |
| 1 | Determinism and LAN multiplayer | 4–6 weeks |
| 2 | Internet play via relay | 3–4 weeks |
| 3 | Core gameplay, placeholder art | 3–5 months |
| 4 | Art, audio, feel | 4–8 months |
| 5 | Full rosters and the new country | 2–4 months |
| 6 | Release engineering | 1–2 months |

**Realistically 18–30 months part-time.** Phases 0–2 are roughly three months and produce a
networked, playable prototype — that is the milestone worth aiming at first, because it converts
the project from an idea into something demonstrably real.

The estimates for phases 4 and 5 are the least certain, because they scale with art volume
rather than with engineering. A reduced initial roster is the obvious lever if the schedule
needs to shorten.
