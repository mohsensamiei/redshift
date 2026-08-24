//! Grid pathfinding.
//!
//! A\* over the eight-connected grid, with two properties that matter more
//! here than raw speed:
//!
//! # Determinism
//!
//! Every peer must find the *same* path, not merely an equally good one. Two
//! routes of identical cost are common on an open grid, so the tie-break is
//! part of the game's behaviour:
//!
//! 1. lowest `f` score,
//! 2. then lowest cell index.
//!
//! That is a total order, so the frontier can never be ordered differently on
//! two machines. Costs are integers throughout — 10 for an orthogonal step and
//! 14 for a diagonal, the usual approximation of 1 and √2 — so no rounding can
//! disagree either.
//!
//! # A budget in nodes, never in milliseconds
//!
//! Pathfinding is the dominant CPU cost in an RTS, so it must be bounded. It is
//! bounded by a count of node expansions, never by elapsed time: a time-based
//! cutoff produces different results on a fast and a slow machine, which is the
//! single most common way an RTS desyncs.
//!
//! When the budget runs out, the search returns the best partial route it found
//! — the reachable cell closest to the goal. The unit walks that far and asks
//! again, which makes progress without ever needing to keep a half-finished
//! search alive between ticks.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::map::{Cell, Map, SurfaceMask};

/// Cost of an orthogonal step.
pub const COST_STRAIGHT: u32 = 10;
/// Cost of a diagonal step: 10 × √2, rounded.
pub const COST_DIAGONAL: u32 = 14;

/// Node expansions a single search may spend before giving up and returning a
/// partial route.
///
/// Generous enough to solve any ordinary route on a mid-sized map, small enough
/// that a unit ordered into a sealed room cannot stall the tick.
pub const DEFAULT_NODE_BUDGET: u32 = 4_000;

/// The outcome of a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathResult {
    /// A complete route to the goal. Excludes the starting cell; the last entry
    /// is the goal.
    Found(Vec<Cell>),
    /// The budget ran out before the goal was found. The route leads to the
    /// most promising reachable cell; walk it and ask again.
    ///
    /// This says nothing about whether the goal is reachable — only that this
    /// search could not afford to find out. A caller that keeps receiving
    /// `Partial` without arriving should raise the budget rather than give up,
    /// since a concave obstacle can require exploring away from the goal before
    /// making progress towards it.
    ///
    /// May be empty if the budget was too small to leave the starting cell.
    Partial(Vec<Cell>),
    /// The goal cannot be reached. The search proved it by exhausting every
    /// reachable cell, so this is a fact and not a budget artefact.
    Unreachable,
}

impl PathResult {
    /// The route, whether complete or partial.
    pub fn cells(&self) -> &[Cell] {
        match self {
            PathResult::Found(c) | PathResult::Partial(c) => c,
            PathResult::Unreachable => &[],
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, PathResult::Found(_))
    }
}

/// A frontier entry.
///
/// `Ord` is `(f, cell)` — the tie-break that makes the search deterministic.
/// Deriving it relies on field order, so the fields must stay in this order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct OpenNode {
    f: u32,
    cell: u32,
}

/// Reusable scratch space for pathfinding.
///
/// One of these is shared by every search in a match rather than allocated per
/// request: a per-search allocation of map-sized arrays would cost hundreds of
/// megabytes with a few hundred units pathing at once.
///
/// The `stamp` array avoids clearing between searches — an entry is stale
/// unless its stamp matches the current generation — which keeps each search
/// proportional to the cells it actually visits, not to the map size.
#[derive(Clone, Debug)]
pub struct PathWorkspace {
    g: Vec<u32>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    generation: u32,
    open: BinaryHeap<Reverse<OpenNode>>,
    /// Node expansions spent by the most recent search. Diagnostics only.
    last_expansions: u32,
}

impl PathWorkspace {
    pub fn new(cell_count: usize) -> PathWorkspace {
        PathWorkspace {
            g: vec![0; cell_count],
            parent: vec![u32::MAX; cell_count],
            stamp: vec![0; cell_count],
            generation: 0,
            open: BinaryHeap::new(),
            last_expansions: 0,
        }
    }

    /// Node expansions spent by the most recent search.
    pub fn last_expansions(&self) -> u32 {
        self.last_expansions
    }

    fn begin(&mut self, cell_count: usize) {
        if self.g.len() != cell_count {
            self.g.resize(cell_count, 0);
            self.parent.resize(cell_count, u32::MAX);
            self.stamp.resize(cell_count, 0);
        }
        self.open.clear();
        // Wrapping into a generation that still marks cells as visited would
        // make a search read another search's results. Clearing on wrap costs
        // one pass every four billion searches.
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.generation = 1;
        }
    }

    #[inline]
    fn visited(&self, index: u32) -> bool {
        self.stamp[index as usize] == self.generation
    }
}

/// The octile heuristic, matching the step costs exactly.
///
/// Admissible and consistent: it never overestimates, so A\* is optimal, and it
/// agrees with the real cost on open ground so the search does not fan out
/// needlessly.
#[inline]
fn heuristic(from: Cell, to: Cell) -> u32 {
    let dx = (from.x - to.x).unsigned_abs();
    let dy = (from.y - to.y).unsigned_abs();
    let (lo, hi) = if dx < dy { (dx, dy) } else { (dy, dx) };
    COST_STRAIGHT * (hi - lo) + COST_DIAGONAL * lo
}

/// Finds a route from `start` to `goal`.
///
/// The returned route excludes `start`. An unreachable goal costs a full search
/// of the connected region, which is exactly what the node budget is there to
/// bound.
pub fn find_path(
    map: &Map,
    workspace: &mut PathWorkspace,
    start: Cell,
    goal: Cell,
    movement: SurfaceMask,
    node_budget: u32,
) -> PathResult {
    workspace.last_expansions = 0;

    let (Some(start_idx), Some(goal_idx)) = (map.index(start), map.index(goal)) else {
        return PathResult::Unreachable;
    };
    if start == goal {
        return PathResult::Found(Vec::new());
    }
    if !map.is_passable(start, movement) {
        // A unit standing somewhere it cannot legally be — pushed there by a
        // later collision pass, or spawned badly. Refusing here rather than
        // searching from an invalid cell keeps the failure visible.
        return PathResult::Unreachable;
    }

    workspace.begin(map.cell_count());

    // Tracks the best partial answer, so an exhausted budget still yields
    // forward progress rather than nothing.
    let mut best_idx = start_idx;
    let mut best_h = heuristic(start, goal);

    workspace.g[start_idx as usize] = 0;
    workspace.parent[start_idx as usize] = u32::MAX;
    workspace.stamp[start_idx as usize] = workspace.generation;
    workspace.open.push(Reverse(OpenNode {
        f: best_h,
        cell: start_idx,
    }));

    let mut expansions = 0u32;
    let mut reached_goal = false;
    let mut budget_exhausted = false;
    // The cell the search was about to expand when the budget ran out. Used as
    // a fallback route when no cell beat the starting heuristic — which happens
    // whenever the unit sits in a pocket and must move away from the goal
    // before it can move towards it.
    let mut frontier_idx = start_idx;

    while let Some(Reverse(node)) = workspace.open.pop() {
        if node.cell == goal_idx {
            reached_goal = true;
            break;
        }

        // A cell can be pushed more than once when a cheaper route to it is
        // found. The stale copies are skipped here rather than removed from the
        // heap, which a binary heap cannot do cheaply.
        let current_g = workspace.g[node.cell as usize];
        if node.f < current_g {
            continue;
        }

        expansions += 1;
        if expansions >= node_budget {
            budget_exhausted = true;
            frontier_idx = node.cell;
            break;
        }

        let cell = map.cell_at(node.cell);
        for next in cell.neighbours() {
            if !map.is_passable(next, movement) {
                continue;
            }
            // A cliff blocks the step rather than the cell. High ground is
            // somewhere a unit can stand and fight; what it cannot do is walk
            // up the side.
            if !map.step_is_climbable(cell, next, movement) {
                continue;
            }
            if !map.allows_diagonal(cell, next, movement) {
                continue;
            }
            let Some(next_idx) = map.index(next) else {
                continue;
            };

            let diagonal = cell.x != next.x && cell.y != next.y;
            let step = if diagonal {
                COST_DIAGONAL
            } else {
                COST_STRAIGHT
            };
            let terrain_pct = 100;
            let step = step.saturating_mul(terrain_pct) / 100;
            let tentative = current_g + step;

            let seen = workspace.visited(next_idx);
            if seen && tentative >= workspace.g[next_idx as usize] {
                continue;
            }

            workspace.g[next_idx as usize] = tentative;
            workspace.parent[next_idx as usize] = node.cell;
            workspace.stamp[next_idx as usize] = workspace.generation;

            let h = heuristic(next, goal);
            if h < best_h {
                best_h = h;
                best_idx = next_idx;
            }
            workspace.open.push(Reverse(OpenNode {
                f: tentative + h,
                cell: next_idx,
            }));
        }
    }

    workspace.last_expansions = expansions;

    if reached_goal {
        return PathResult::Found(reconstruct(workspace, start_idx, goal_idx, map));
    }

    if budget_exhausted {
        // Running out of budget says nothing about connectivity, so this must
        // never be reported as unreachable — a unit in a concave pocket would
        // abandon a perfectly valid order.
        //
        // Prefer the cell that came closest to the goal. Failing that, head for
        // the frontier the search was about to expand: it is still a real,
        // reachable cell, and moving there lets the next search continue from
        // further out instead of re-treading the same ground.
        let end = if best_idx != start_idx {
            best_idx
        } else {
            frontier_idx
        };
        return PathResult::Partial(reconstruct(workspace, start_idx, end, map));
    }

    // The frontier emptied: every reachable cell was examined and none was the
    // goal. This is a proof, not a guess.
    PathResult::Unreachable
}

/// Walks parent links back from `end` to `start` and reverses them.
fn reconstruct(workspace: &PathWorkspace, start_idx: u32, end_idx: u32, map: &Map) -> Vec<Cell> {
    let mut cells = Vec::new();
    let mut current = end_idx;
    while current != start_idx {
        cells.push(map.cell_at(current));
        let parent = workspace.parent[current as usize];
        debug_assert_ne!(
            parent,
            u32::MAX,
            "parent chain broke before reaching the start"
        );
        if parent == u32::MAX {
            break;
        }
        current = parent;
    }
    cells.reverse();
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a map split by a wall of rock, with a channel of water crossing
    /// it — so land, sea and air each have a different answer.
    ///
    /// ```text
    ///        x=0        x=5(rock)      x=11
    ///   y=0  . . . . .  #  . . . . .
    ///   y=3  ~ ~ ~ ~ ~  ~  ~ ~ ~ ~ ~   <- water channel through the wall
    ///   y=6  . . . . .  #  . . . . .
    /// ```
    fn divided_map() -> Map {
        let mut map = Map::new(12, 8);
        map.fill_rect(Cell::new(5, 0), Cell::new(5, 7), Terrain::Rock);
        map.fill_rect(Cell::new(0, 3), Cell::new(11, 3), Terrain::Water);
        map
    }

    #[test]
    fn a_ship_stays_on_water_and_a_tank_stays_off_it() {
        let map = divided_map();
        let mut workspace = PathWorkspace::new(map.cell_count());

        // The ship follows the channel straight through the wall.
        let ship = find_path(
            &map,
            &mut workspace,
            Cell::new(1, 3),
            Cell::new(10, 3),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Water]),
            10_000,
        );
        let route = match ship {
            PathResult::Found(cells) => cells,
            other => panic!("a ship on open water should find a route, got {other:?}"),
        };
        for cell in &route {
            assert_eq!(
                map.terrain(*cell),
                Terrain::Water,
                "the ship routed through {cell:?}, which is not water"
            );
        }

        // The same journey by land has to go around the water, and cannot
        // cross the wall at all — so it fails outright.
        let tank = find_path(
            &map,
            &mut workspace,
            Cell::new(1, 3),
            Cell::new(10, 3),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            10_000,
        );
        assert!(
            matches!(tank, PathResult::Unreachable),
            "a tank should not be able to start on water, got {tank:?}"
        );
    }

    #[test]
    fn aircraft_fly_over_everything_in_the_way() {
        let map = divided_map();
        let mut workspace = PathWorkspace::new(map.cell_count());

        let air = find_path(
            &map,
            &mut workspace,
            Cell::new(1, 1),
            Cell::new(10, 6),
            SurfaceMask::from_surfaces(&[
                crate::map::Surface::Land,
                crate::map::Surface::Water,
                crate::map::Surface::Height,
            ]),
            10_000,
        );
        let route = match air {
            PathResult::Found(cells) => cells,
            other => panic!("aircraft should always find a route, got {other:?}"),
        };

        // The route is direct: it crosses both the rock wall and the water
        // rather than detouring around either.
        assert!(
            route.iter().any(|c| map.terrain(*c) == Terrain::Rock),
            "the flight path avoided the rock instead of crossing it"
        );

        // And it is no longer than the diagonal distance, which is what
        // ignoring terrain buys.
        let straight = (10i32 - 1).abs().max((6i32 - 1).abs()) as usize;
        assert!(
            route.len() <= straight + 1,
            "the flight took {} steps for a {straight}-step journey",
            route.len()
        );
    }

    #[test]
    fn ground_units_route_around_water_rather_than_through_it() {
        let mut map = Map::new(12, 8);
        // A pond that does not quite reach the map edge.
        map.fill_rect(Cell::new(4, 2), Cell::new(7, 5), Terrain::Water);
        let mut workspace = PathWorkspace::new(map.cell_count());

        let route = match find_path(
            &map,
            &mut workspace,
            Cell::new(1, 4),
            Cell::new(10, 4),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            10_000,
        ) {
            PathResult::Found(cells) => cells,
            other => panic!("there is a way around, got {other:?}"),
        };
        assert!(
            route.iter().all(|c| map.terrain(*c) != Terrain::Water),
            "a tracked unit routed through water"
        );
    }

    #[test]
    fn a_ship_cannot_reach_a_landlocked_destination() {
        // Better to report no route than to path partway and stall against the
        // shore every tick.
        let mut map = Map::new(10, 10);
        map.fill_rect(Cell::new(0, 0), Cell::new(9, 4), Terrain::Water);
        let mut workspace = PathWorkspace::new(map.cell_count());

        let result = find_path(
            &map,
            &mut workspace,
            Cell::new(2, 2),
            Cell::new(5, 8),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Water]),
            10_000,
        );
        assert!(
            matches!(result, PathResult::Unreachable),
            "expected no route to dry land, got {result:?}"
        );
    }
    use crate::map::Terrain;

    fn workspace_for(map: &Map) -> PathWorkspace {
        PathWorkspace::new(map.cell_count())
    }

    fn path(map: &Map, from: (i32, i32), to: (i32, i32)) -> PathResult {
        let mut ws = workspace_for(map);
        find_path(
            map,
            &mut ws,
            Cell::new(from.0, from.1),
            Cell::new(to.0, to.1),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            DEFAULT_NODE_BUDGET,
        )
    }

    /// Verifies a route is actually walkable: every step is to a neighbour, and
    /// every cell is passable. A path that merely ends in the right place is
    /// not enough.
    fn assert_walkable(map: &Map, start: Cell, cells: &[Cell]) {
        let mut prev = start;
        for (i, &c) in cells.iter().enumerate() {
            assert!(
                map.is_passable(c, SurfaceMask::from_surfaces(&[crate::map::Surface::Land])),
                "step {i} enters impassable {c:?}"
            );
            assert_eq!(
                prev.chebyshev_to(c),
                1,
                "step {i} is not a single step: {prev:?} -> {c:?}"
            );
            assert!(
                map.allows_diagonal(
                    prev,
                    c,
                    SurfaceMask::from_surfaces(&[crate::map::Surface::Land])
                ),
                "step {i} cuts a corner: {prev:?} -> {c:?}"
            );
            prev = c;
        }
    }

    #[test]
    fn straight_line_on_open_ground() {
        let map = Map::new(16, 16);
        let result = path(&map, (2, 2), (8, 2));
        assert!(result.is_complete());
        assert_eq!(result.cells().len(), 6, "six steps east");
        assert_eq!(*result.cells().last().unwrap(), Cell::new(8, 2));
        assert_walkable(&map, Cell::new(2, 2), result.cells());
    }

    #[test]
    fn diagonal_costs_less_than_going_around() {
        let map = Map::new(16, 16);
        let result = path(&map, (0, 0), (5, 5));
        assert!(result.is_complete());
        // Pure diagonal: five steps, not ten.
        assert_eq!(result.cells().len(), 5);
    }

    #[test]
    fn path_to_self_is_empty_but_successful() {
        let map = Map::new(8, 8);
        let result = path(&map, (3, 3), (3, 3));
        assert_eq!(result, PathResult::Found(Vec::new()));
    }

    #[test]
    fn routes_around_a_wall() {
        let mut map = Map::new(16, 16);
        // A wall with a gap at the top.
        map.fill_rect(Cell::new(8, 0), Cell::new(8, 12), Terrain::Rock);

        let result = path(&map, (2, 6), (14, 6));
        assert!(result.is_complete(), "there is a way around");
        assert_walkable(&map, Cell::new(2, 6), result.cells());
        assert!(
            result
                .cells()
                .iter()
                .all(|c| map.terrain(*c) != Terrain::Rock),
            "the route must not pass through the wall"
        );
        assert_eq!(*result.cells().last().unwrap(), Cell::new(14, 6));
    }

    #[test]
    fn sealed_goal_is_proved_unreachable() {
        let mut map = Map::new(16, 16);
        // Enclose the goal completely.
        map.fill_rect(Cell::new(9, 9), Cell::new(11, 11), Terrain::Rock);
        map.set_terrain(Cell::new(10, 10), Terrain::Ground);

        let result = path(&map, (2, 2), (10, 10));
        assert_eq!(result, PathResult::Unreachable);
    }

    #[test]
    fn goal_outside_the_map_is_unreachable() {
        let map = Map::new(8, 8);
        assert_eq!(path(&map, (1, 1), (99, 99)), PathResult::Unreachable);
        assert_eq!(path(&map, (1, 1), (-1, 0)), PathResult::Unreachable);
    }

    #[test]
    fn goal_on_impassable_terrain_is_unreachable() {
        let mut map = Map::new(16, 16);
        map.set_terrain(Cell::new(10, 10), Terrain::Water);
        assert_eq!(path(&map, (2, 2), (10, 10)), PathResult::Unreachable);
    }

    #[test]
    fn start_on_impassable_terrain_is_refused() {
        let mut map = Map::new(16, 16);
        map.set_terrain(Cell::new(2, 2), Terrain::Water);
        assert_eq!(path(&map, (2, 2), (10, 10)), PathResult::Unreachable);
    }

    #[test]
    fn never_cuts_a_wall_corner() {
        let mut map = Map::new(12, 12);
        map.set_terrain(Cell::new(5, 4), Terrain::Rock);
        map.set_terrain(Cell::new(4, 5), Terrain::Rock);

        let result = path(&map, (4, 4), (5, 5));
        assert!(result.is_complete());
        assert_walkable(&map, Cell::new(4, 4), result.cells());
        assert!(
            result.cells().len() > 1,
            "must go around, not squeeze diagonally"
        );
    }

    #[test]
    fn air_ignores_terrain() {
        let mut map = Map::new(16, 16);
        map.fill_rect(Cell::new(8, 0), Cell::new(8, 15), Terrain::Rock);
        let mut ws = workspace_for(&map);

        let ground = find_path(
            &map,
            &mut ws,
            Cell::new(2, 8),
            Cell::new(14, 8),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            DEFAULT_NODE_BUDGET,
        );
        assert_eq!(ground, PathResult::Unreachable, "the wall spans the map");

        let air = find_path(
            &map,
            &mut ws,
            Cell::new(2, 8),
            Cell::new(14, 8),
            SurfaceMask::from_surfaces(&[
                crate::map::Surface::Land,
                crate::map::Surface::Water,
                crate::map::Surface::Height,
            ]),
            DEFAULT_NODE_BUDGET,
        );
        assert!(air.is_complete());
        assert_eq!(air.cells().len(), 12, "straight over the wall");
    }

    #[test]
    fn identical_queries_give_identical_paths() {
        // The core determinism property. On open ground many routes tie on
        // cost; the tie-break must pick the same one every time.
        let mut map = Map::new(32, 32);
        map.fill_rect(Cell::new(10, 4), Cell::new(10, 20), Terrain::Rock);
        map.fill_rect(Cell::new(20, 12), Cell::new(20, 28), Terrain::Rock);

        let first = path(&map, (1, 1), (30, 30));
        for _ in 0..20 {
            assert_eq!(path(&map, (1, 1), (30, 30)), first);
        }
    }

    #[test]
    fn a_reused_workspace_does_not_leak_between_searches() {
        // The stamp-generation trick skips clearing the arrays. If it were
        // wrong, a later search would read an earlier one's costs.
        let mut map = Map::new(24, 24);
        map.fill_rect(Cell::new(12, 0), Cell::new(12, 18), Terrain::Rock);
        let mut shared = workspace_for(&map);

        let mut reference = Vec::new();
        for (from, to) in [((1, 1), (22, 22)), ((3, 20), (20, 3)), ((0, 0), (23, 0))] {
            let mut fresh = workspace_for(&map);
            reference.push(find_path(
                &map,
                &mut fresh,
                Cell::new(from.0, from.1),
                Cell::new(to.0, to.1),
                SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                DEFAULT_NODE_BUDGET,
            ));
        }

        // Run the same queries repeatedly through one shared workspace.
        for _ in 0..3 {
            for (i, (from, to)) in [((1, 1), (22, 22)), ((3, 20), (20, 3)), ((0, 0), (23, 0))]
                .iter()
                .enumerate()
            {
                let got = find_path(
                    &map,
                    &mut shared,
                    Cell::new(from.0, from.1),
                    Cell::new(to.0, to.1),
                    SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                    DEFAULT_NODE_BUDGET,
                );
                assert_eq!(got, reference[i], "shared workspace diverged on query {i}");
            }
        }
    }

    #[test]
    fn generation_wrap_clears_stale_stamps() {
        // Force the wrap path. Without the clear, every cell would look visited
        // and the search would find nothing.
        let map = Map::new(8, 8);
        let mut ws = workspace_for(&map);
        ws.generation = u32::MAX;
        ws.stamp.iter_mut().for_each(|s| *s = u32::MAX);

        let result = find_path(
            &map,
            &mut ws,
            Cell::new(0, 0),
            Cell::new(7, 7),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            DEFAULT_NODE_BUDGET,
        );
        assert!(
            result.is_complete(),
            "search must survive the generation wrap"
        );
    }

    #[test]
    fn a_tight_budget_returns_progress_not_failure() {
        let map = Map::new(64, 64);
        let mut ws = workspace_for(&map);
        let result = find_path(
            &map,
            &mut ws,
            Cell::new(1, 1),
            Cell::new(62, 62),
            SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
            20,
        );

        match &result {
            PathResult::Partial(cells) => {
                assert!(
                    !cells.is_empty(),
                    "a partial route must still make progress"
                );
                assert_walkable(&map, Cell::new(1, 1), cells);
                let end = *cells.last().unwrap();
                assert!(
                    end.chebyshev_to(Cell::new(62, 62))
                        < Cell::new(1, 1).chebyshev_to(Cell::new(62, 62)),
                    "the partial route must end closer to the goal than the start was"
                );
            }
            other => panic!("expected a partial route, got {other:?}"),
        }
        assert!(ws.last_expansions() <= 20, "the budget must be respected");
    }

    #[test]
    fn budget_is_counted_in_nodes_not_time() {
        // Same query, same budget, same expansions — regardless of machine
        // speed. A time-based budget could not promise this.
        let map = Map::new(48, 48);
        let mut ws = workspace_for(&map);
        let mut counts = Vec::new();
        for _ in 0..5 {
            find_path(
                &map,
                &mut ws,
                Cell::new(1, 1),
                Cell::new(46, 46),
                SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                500,
            );
            counts.push(ws.last_expansions());
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "expansions varied: {counts:?}"
        );
    }

    #[test]
    fn repeated_partial_paths_eventually_arrive() {
        // What makes partial routes safe: a caller that walks the route and
        // raises its budget on each retry converges. Escalation is the caller's
        // job — a fixed small budget can livelock in concave terrain, which is
        // why `Partial` explicitly does not mean "give up".
        let mut map = Map::new(64, 64);
        map.fill_rect(Cell::new(20, 0), Cell::new(20, 50), Terrain::Rock);
        map.fill_rect(Cell::new(40, 12), Cell::new(40, 63), Terrain::Rock);

        let mut ws = workspace_for(&map);
        let goal = Cell::new(60, 60);
        let mut at = Cell::new(1, 1);
        let mut budget = 40u32;

        for hop in 0..100 {
            match find_path(
                &map,
                &mut ws,
                at,
                goal,
                SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                budget,
            ) {
                PathResult::Found(cells) => {
                    assert_walkable(&map, at, &cells);
                    assert_eq!(*cells.last().unwrap(), goal);
                    return;
                }
                PathResult::Partial(cells) => {
                    assert_walkable(&map, at, &cells);
                    if let Some(&next) = cells.last() {
                        at = next;
                    }
                    budget = (budget * 2).min(DEFAULT_NODE_BUDGET);
                }
                PathResult::Unreachable => panic!("hop {hop}: the goal is reachable"),
            }
        }
        panic!("did not converge within 100 hops; reached {at:?}");
    }

    #[test]
    fn an_exhausted_budget_is_never_reported_as_unreachable() {
        // The distinction this test pins: "I could not afford to find out" is
        // not "there is no way". Conflating them makes units in a pocket
        // silently abandon valid orders.
        let mut map = Map::new(64, 64);
        // A pocket whose only exit faces west, while the goal lies far to the
        // north-east: every cell the search can reach early scores worse than
        // the start, so `best_idx` never improves.
        map.fill_rect(Cell::new(8, 8), Cell::new(14, 14), Terrain::Rock);
        map.fill_rect(Cell::new(10, 10), Cell::new(12, 12), Terrain::Ground);
        map.set_terrain(Cell::new(9, 11), Terrain::Ground);
        map.set_terrain(Cell::new(8, 11), Terrain::Ground);

        let mut ws = workspace_for(&map);

        // The goal really is reachable, given enough budget.
        assert!(
            find_path(
                &map,
                &mut ws,
                Cell::new(11, 11),
                Cell::new(60, 60),
                SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                DEFAULT_NODE_BUDGET,
            )
            .is_complete(),
            "test map is wrong: the pocket must have a way out"
        );

        for budget in [2, 3, 5, 8, 13, 21, 34] {
            let result = find_path(
                &map,
                &mut ws,
                Cell::new(11, 11),
                Cell::new(60, 60),
                SurfaceMask::from_surfaces(&[crate::map::Surface::Land]),
                budget,
            );
            assert_ne!(
                result,
                PathResult::Unreachable,
                "budget {budget} was reported as unreachable"
            );
        }
    }

    #[test]
    fn found_routes_are_optimal() {
        // Spot-check against the octile cost, which is exact on open ground.
        let map = Map::new(32, 32);
        for (from, to) in [((0, 0), (10, 0)), ((0, 0), (7, 7)), ((3, 1), (20, 9))] {
            let start = Cell::new(from.0, from.1);
            let goal = Cell::new(to.0, to.1);
            let result = path(&map, from, to);
            assert!(result.is_complete());

            let cost: u32 = result
                .cells()
                .iter()
                .fold((start, 0u32), |(prev, acc), &c| {
                    let diagonal = prev.x != c.x && prev.y != c.y;
                    (
                        c,
                        acc + if diagonal {
                            COST_DIAGONAL
                        } else {
                            COST_STRAIGHT
                        },
                    )
                })
                .1;
            assert_eq!(
                cost,
                heuristic(start, goal),
                "route from {from:?} to {to:?} is not optimal"
            );
        }
    }
}
