# 03 — Networking

LAN and internet play share almost all of their code. The only difference is how packets get
from A to B. This is a direct consequence of the lockstep model.

## Model: deterministic lockstep

Peers exchange **commands**, not world state.

```
Player clicks ──▶ Command ──▶ scheduled for tick N+D ──▶ broadcast to all peers
                                                              │
   all peers collect every player's commands for tick N+D ◀────┘
                                                              │
                      each peer runs sim.tick(commands) locally
                                                              │
                      every 20 ticks: exchange state_hash ────┘
```

A command is small — a verb, a few entity ids, a target position. A 200-unit army move is one
command, not 200 position updates. Bandwidth is therefore a few hundred bytes per second per
player, independent of army size, and identical on LAN and internet.

The price is that **every peer must simulate identically**. That requirement is documented in
[02-simulation.md](02-simulation.md) and is why the sim rules are so strict.

### Command scheduling

Commands issued at tick `N` execute at tick `N + D`, where `D` is the **input delay** — enough
ticks for the command to reach every peer. At 20 Hz, one tick is 50 ms.

| Delay `D` | Latency budget | Feels like |
|---|---|---|
| 2 ticks | 100 ms | LAN |
| 4 ticks | 200 ms | good internet |
| 6 ticks | 300 ms | tolerable internet |

`D` is negotiated at match start from measured RTT and stays fixed for the match. Renegotiating
mid-match is possible but deferred — a fixed `D` is simpler and keeps replays trivially valid.

Input delay is not lag: the *unit* responds late, but the interface responds instantly. The
renderer plays the selection sound and shows the move marker immediately, on the frame of the
click. This is what the original did, and it is why 200 ms of input delay is nearly invisible
in an RTS while being unplayable in a shooter.

### Turn advancement

A peer may only run tick `N` once it has every player's command packet for tick `N`. If a
packet is missing, the sim **waits** — it never extrapolates or guesses, because a wrong guess
is a desync. A brief stall is shown as a "waiting for player" indicator after ~500 ms.

Peers send a packet every tick even when the player did nothing (an empty command set), so a
silent player is distinguishable from a disconnected one.

## Transport

UDP throughout, with a thin reliability layer: sequence numbers, and each packet redundantly
carries the last few ticks' commands. Because commands are tiny, resending the last 3 ticks in
every packet is cheaper than implementing retransmission requests, and it makes single packet
loss invisible.

## LAN play

Zero configuration is a requirement: two machines on the same Wi-Fi should simply see each other.

- **Discovery:** the host broadcasts a small announce packet on UDP port `47654` (subject to
  change) roughly twice a second: match name, map, player count, protocol version. Clients listen
  and populate a live list. No mDNS/Bonjour dependency — plain broadcast is simpler and has no
  platform-specific behaviour to debug.
- **Connection:** peer-to-peer UDP directly between the machines. No server, no internet needed.
- **Protocol version** is in the announce packet; mismatched versions are shown as incompatible
  rather than being allowed to connect and desync.

## Internet play

Direct peer-to-peer across the internet fails for most players because of NAT. Rather than
implementing NAT traversal (which works unreliably and needs fallback infrastructure anyway),
we go straight to a **relay**.

`redshift-server` provides two services:

### Lobby

Tracks open matches and lets clients create/join them. Small HTTP/WebSocket service. Knows match
metadata only — name, map, player slots, protocol version.

### Relay

Forwards command packets between the peers of a match. It is **a switch, not an authority**:

- It does not simulate the game.
- It does not know the rules.
- It cannot desync, because it holds no game state.

This is deliberate. It makes the server tiny, cheap to host (a small VPS handles many concurrent
matches, since traffic is a few hundred bytes/sec per player), and means server bugs cannot
corrupt a match. It is the same architecture the long-running community servers for this era of
games have used successfully for years.

Direct P2P is attempted first and the relay is the fallback, so players on the same network or
with cooperative NATs get the lower latency path automatically.

## Desync detection

Assume desyncs will happen during development. The goal is to catch them within one second of
occurring, not ten minutes later.

- Every 20 ticks each peer sends `state_hash()` for a recent tick alongside its command packet.
- On mismatch: the match halts immediately with a clear message. Both peers write a dump —
  full sim state, plus the command log for the whole match.
- The command log alone reproduces the divergence offline: replay both dumps in one process,
  tick until the hashes differ, and bisect to the exact tick and entity.

For development builds, an optional high-frequency mode hashes every tick and hashes per
subsystem, so the dump points at *which phase* diverged rather than just *when*.

## Replays

A replay is the match seed, the rules version, and the command log. That is all — a full match
is a few kilobytes. Playback is just running the sim against the recorded commands with no
network.

This falls out of lockstep for free and is worth building in Phase 1 rather than later, because
it is the primary debugging tool for the rest of the project.

## Reconnection and spectators

Both are Phase 2 work, and both are straightforward given the above:

- A **spectator** is a peer that receives commands and simulates, but sends none.
- A **reconnecting player** receives a state snapshot plus the commands since that snapshot, then
  catches up by simulating at maximum speed.

## Security posture

Lockstep with no server authority means a modified client can cheat by revealing the fog of war
(it holds the full world state locally). This is inherent to the model and was true of the
original game too. Mitigations if it matters later: replay analysis, and hash-checking the rules
files at match start so clients cannot silently alter unit stats. Full anti-cheat is out of scope.
