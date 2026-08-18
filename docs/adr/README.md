# Architecture Decision Records

Each ADR records one significant decision: the context it was made in, what was decided, and the
consequences that follow. They exist so that a decision made once does not get silently re-litigated,
and so that anyone joining later can understand *why* the code looks the way it does.

## Conventions

- One file per decision, numbered sequentially: `NNNN-short-title.md`.
- **Never delete or rewrite history.** A decision that changes gets a *new* ADR that supersedes the
  old one; the old file stays and is marked `Superseded by NNNN`.
- Status is one of `Proposed`, `Accepted`, `Superseded by NNNN`, or `Deprecated`.
- Consequences matter more than the decision itself. Record the costs honestly, including the ones
  we chose to accept.

## Index

| # | Decision | Status |
|---|---|---|
| [0001](0001-rust-and-bevy.md) | Rust with Bevy for presentation, custom simulation | Accepted |
| [0002](0002-realtime-3d-under-a-budget.md) | Real-time 3D under a hard performance budget | Accepted |
| [0003](0003-deterministic-lockstep.md) | Deterministic lockstep networking | Accepted |
| [0004](0004-original-assets-only.md) | Original assets only | Accepted |
| [0005](0005-faithful-remaster-scope.md) | Faithful remaster, not a redesign | Accepted |
| [0006](0006-capability-is-data-not-category.md) | Capability belongs to the unit, never to its category | Accepted |

## The load-bearing ones

If you read only three: **0003** constrains the entire codebase (it is why floats are banned in
the simulation), **0005** is what keeps the scope finite, and **0006** is why a unit's abilities
live in its rules file rather than in a `match` on what kind of unit it is.
