# ADR 0005 — Faithful remaster, not a redesign

**Status:** Accepted · **Date:** 2026-08-15

## Context

Once the engine is being written from scratch, there is a constant temptation to "improve" the
game design along the way — modernise the economy, smooth the pacing, rebalance the counters, add
quality-of-life mechanics from newer strategy games.

The stated requirement is the opposite: *"I don't want a new game."* The nostalgic design must
be preserved.

## Decision

**Gameplay design is treated as fixed and already-specified.** Unit roles, economy, build order,
pacing, damage relationships and control feel follow the original. We modernise the engine and
the presentation only.

Practical rule: when a design question comes up, the answer is *"what did the original do?"* —
not *"what would be better?"* If the original's behaviour is unclear, that is flagged rather than
resolved by invention.

Balance changes are deferred until the faithful baseline is playable and proven. They may never
be made at all.

Modernisation is permitted only where it does not alter game outcomes:

**Allowed** — higher resolution and smooth zoom; larger selection limits; better group and hotkey
handling; readable modern UI scaling; smooth interpolated motion; improved pathfinding *quality*
where it does not change unit speed or arrival timing.

**Not allowed** — changed unit stats, costs or build times; new mechanics; altered economy rates;
free camera rotation; changed tech tree structure.

The original's own countries are in scope, each with its unique unit or structure and its
passive modifier. *Additional* countries are not — see below.

## The country roster is the original's, exactly

Stated separately because it is the rule most likely to be quietly bent.

**The shipped remaster has the same countries the original had — no more, no
fewer.** They are:

| Side | Countries |
|---|---|
| Allied | America, Korea, France, Germany, Great Britain |
| Soviet | Russia, Iraq, Libya, Cuba |

Each keeps its own unique unit or structure and its own passive advantage, as
in the original.

New countries are wanted, and are explicitly **not part of the remaster**. They
come after it ships, as the first entry in a separate content phase. The
reasoning is the same one that put netcode before art: a remaster that is
"nearly faithful plus some new things" can never be checked against anything,
because there is no longer a version of the game to compare it to. Finish the
faithful one, confirm it plays like the original, and then add.

This is not a judgement about the new countries. It is about having a finished,
comparable baseline before changing it.

## Consequences

- **The single largest risk in game development is removed.** Game design is the part that most
  often fails, requires many iterations, and cannot be validated except by playtesting. Here it
  is already designed, shipped, and proven over decades. This is a substantial schedule and risk
  advantage and should not be given back.
- Design debates are short: the question is factual (what did it do?) rather than a matter of
  taste.
- Pathfinding, unit turning and firing timings must be tuned to *feel* like the original, which
  is a real engineering constraint — "faithful" applies to feel, not only to numbers.
- Some original behaviours are, by modern standards, awkward. They are kept anyway unless they
  are outright bugs. Nostalgia is a stated project goal and lives partly in those quirks.
- Any future rebalancing must be opt-in — a separate ruleset, never a change to the default.

## Consequence for the roadmap

Because the design is settled, Phase 3 is an *implementation* task with known requirements rather
than an exploratory one. That is why it carries a narrower estimate than its size would otherwise
suggest, and why the widest uncertainty sits in the art phases instead.
