# ADR 0001 — Rust with Bevy for presentation, custom simulation

**Status:** Accepted · **Date:** 2026-08-15

## Context

We need a stack for a cross-platform isometric RTS. The stated requirements are: runs smoothly
and *quietly* on an Apple M1 Pro with integrated graphics, low resource usage, and a clean,
modern, tidy implementation. Deterministic lockstep multiplayer is a hard requirement
(see [ADR 0003](0003-deterministic-lockstep.md)).

Options considered:

| Option | For | Against |
|---|---|---|
| **Rust + Bevy** | No GC; wgpu targets Metal natively; ECS suits RTS; fixed-point is natural; single small binary | Bevy's API churns between releases; steeper learning curve |
| **Godot 4 (C#)** | Batteries included — editor, UI, audio, asset import; fast to first result | C# GC pauses; Vulkan-via-MoltenVK on macOS adds overhead; engine coupling |
| **TypeScript + Three.js** | Runs everywhere; trivial distribution; proven for this exact game by Chrono Divide | GC pauses; weaker performance ceiling; poor fit for a desktop-only, low-resource target |
| **C++ from scratch** | Maximum control | Slowest path; most boilerplate; no safety benefit over Rust |
| **Fork OpenRA** | Netcode, pathfinding, AI and mod system already exist | 2D sprite renderer with no path to clean 3D; would have to fight the engine for our main goal |

## Decision

**Rust**, with **Bevy 0.19** for the presentation layer only, and a **custom simulation crate**
that has no engine dependency whatsoever.

Rationale for Rust:

- **No garbage collector.** In an RTS with hundreds of units, GC pauses are exactly what makes a
  game feel unsmooth. This eliminated Godot/C# and the browser options against the "روان" requirement.
- **wgpu reaches Metal directly** on Apple Silicon, with no translation layer.
- **Fixed-point arithmetic is natural and enforceable.** Determinism forbids floats in the sim
  ([ADR 0003](0003-deterministic-lockstep.md)); Rust's type system makes that a compile-time
  property rather than a matter of discipline.
- Small static binaries, low memory, no runtime.

Rationale for Bevy specifically:

- It is the only mature Rust engine with a full renderer, asset pipeline (including glTF), input,
  audio and UI. Building those on raw wgpu would cost months for no benefit.
- Its ECS and instanced rendering suit the "many similar units" workload.
- We use only a small part of it, and we deliberately avoid its PBR path
  (see [04-rendering.md](../04-rendering.md)).

Rationale for keeping the sim out of Bevy:

- Bevy's scheduler parallelises systems, which is exactly what determinism forbids.
- Bevy's API changes substantially between releases. Isolating the sim means an engine upgrade
  is a rendering-layer task, never a gameplay-correctness risk.
- It gives us the headless server, replays and testability for free.

## Consequences

- The Bevy version is pinned in `Cargo.toml`. Upgrades are deliberate, scoped to
  `redshift-render`, and never bundled with gameplay changes.
- `cargo tree -p redshift-sim` must show no engine dependency. This is checked in CI.
- We accept writing our own ECS-lite, pathfinding and gameplay systems rather than inheriting
  them from a game framework. Given the sim must be custom for determinism reasons anyway, this
  costs little that we would not have paid regardless.
- Contributors need Rust familiarity. This narrows the potential contributor pool; accepted.

## Rejected alternative worth revisiting

If art production (Phase 4) becomes the schedule bottleneck and engineering capacity is idle,
Godot's editor tooling would have been an advantage. This is not sufficient to reverse the
decision, but it argues for investing early in our own in-editor tooling for map and rules
authoring.
