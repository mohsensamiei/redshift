//! The world grid: cells, terrain, and the coordinate system.
//!
//! # Coordinates
//!
//! Two spaces, deliberately distinct types so they cannot be confused:
//!
//! - [`Cell`] — integer grid coordinates. What pathfinding works in.
//! - [`WorldPos`] — continuous [`Fx`] position, where `1.0` is one cell. What
//!   units actually occupy.
//!
//! A cell's centre is at `(x + 0.5, y + 0.5)` in world space. Units sit at
//! cell centres when stationary, which keeps formations tidy and makes the
//! conversion between the two spaces unambiguous.

use serde::{Deserialize, Serialize};

use crate::fx::{Angle, Fx, FxWide};
use crate::hash::{StateHash, StateHasher};

/// Integer grid coordinates.
///
/// `i32` rather than a smaller type because intermediate arithmetic — a
/// neighbour one step off the edge, a delta between distant cells — must not
/// wrap. Bounds are checked when converting to an index, not by the type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

impl Cell {
    pub const fn new(x: i32, y: i32) -> Cell {
        Cell { x, y }
    }

    /// The world position of this cell's centre.
    #[inline]
    pub fn centre(self) -> WorldPos {
        WorldPos {
            x: Fx::from_int(self.x) + Fx::HALF,
            y: Fx::from_int(self.y) + Fx::HALF,
        }
    }

    /// Chebyshev distance: the number of steps when diagonals are allowed.
    #[inline]
    pub fn chebyshev_to(self, other: Cell) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }

    /// Manhattan distance.
    #[inline]
    pub fn manhattan_to(self, other: Cell) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }

    /// The eight neighbours, in a fixed order.
    ///
    /// The order is part of the determinism contract: it decides which of two
    /// equal-cost paths a search finds first. Changing it changes unit
    /// behaviour and invalidates recorded replays.
    #[inline]
    pub fn neighbours(self) -> [Cell; 8] {
        [
            Cell::new(self.x + 1, self.y),
            Cell::new(self.x + 1, self.y + 1),
            Cell::new(self.x, self.y + 1),
            Cell::new(self.x - 1, self.y + 1),
            Cell::new(self.x - 1, self.y),
            Cell::new(self.x - 1, self.y - 1),
            Cell::new(self.x, self.y - 1),
            Cell::new(self.x + 1, self.y - 1),
        ]
    }
}

impl StateHash for Cell {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_i32(self.x);
        h.write_i32(self.y);
    }
}

/// A continuous position in world space, in cell units.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct WorldPos {
    pub x: Fx,
    pub y: Fx,
}

impl WorldPos {
    pub const ORIGIN: WorldPos = WorldPos {
        x: Fx::ZERO,
        y: Fx::ZERO,
    };

    pub const fn new(x: Fx, y: Fx) -> WorldPos {
        WorldPos { x, y }
    }

    /// The cell containing this position.
    ///
    /// Uses floor, not truncation: truncating would fold `-0.5` and `0.5` into
    /// the same cell, making the row and column at the origin twice as wide as
    /// every other.
    #[inline]
    pub fn cell(self) -> Cell {
        Cell::new(self.x.floor_int(), self.y.floor_int())
    }

    #[inline]
    pub fn offset(self, dx: Fx, dy: Fx) -> WorldPos {
        WorldPos {
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    /// Squared distance to another position. Prefer this over [`WorldPos::dist`]
    /// when comparing — it avoids a square root.
    #[inline]
    pub fn dist_sq(self, other: WorldPos) -> FxWide {
        Fx::dist_sq(other.x - self.x, other.y - self.y)
    }

    #[inline]
    pub fn dist(self, other: WorldPos) -> Fx {
        self.dist_sq(other).sqrt()
    }

    /// The heading from this position towards `other`.
    ///
    /// Returns `None` when the two coincide, since no direction is meaningful
    /// there — the caller decides whether to keep its current facing.
    #[inline]
    pub fn heading_to(self, other: WorldPos) -> Option<Angle> {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        if dx == Fx::ZERO && dy == Fx::ZERO {
            None
        } else {
            Some(Angle::from_vector(dx, dy))
        }
    }

    /// Moves `distance` along `heading`.
    #[inline]
    pub fn step(self, heading: Angle, distance: Fx) -> WorldPos {
        WorldPos {
            x: self.x + heading.cos().mul(distance),
            y: self.y + heading.sin().mul(distance),
        }
    }
}

impl StateHash for WorldPos {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_i32(self.x.raw());
        h.write_i32(self.y.raw());
    }
}

/// What a cell is made of.
///
/// Phase 0 keeps this minimal. Height levels, cliffs, bridges and buildable
/// surfaces arrive with the map format in Phase 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Terrain {
    #[default]
    Ground = 0,
    Water = 1,
    /// Impassable to everything: cliff faces, solid rock.
    Rock = 2,
}

/// How a unit traverses terrain.
///
/// Defined in `redshift-data` because rules files name it, and re-exported here
/// so simulation code does not need to reach across for it. A second copy with
/// a conversion between them would be one refactor away from disagreeing.
pub use redshift_data::traits::Locomotor;

/// Terrain rules for each locomotor.
///
/// A free function rather than an inherent method, since [`Locomotor`] belongs
/// to another crate. Which terrain each one may enter becomes data itself once
/// maps carry more than three surfaces.
pub trait TerrainRules {
    fn can_enter(self, terrain: Terrain) -> bool;
    fn cost_percent(self, terrain: Terrain) -> u32;
}

impl TerrainRules for Locomotor {
    #[inline]
    fn can_enter(self, terrain: Terrain) -> bool {
        match self {
            Locomotor::Air => true,
            Locomotor::Ship => terrain == Terrain::Water,
            // Hover crosses water as well as land, which is exactly what makes
            // it worth being a separate locomotor.
            Locomotor::Hover => matches!(terrain, Terrain::Ground | Terrain::Water),
            Locomotor::Foot | Locomotor::Wheeled | Locomotor::Tracked => terrain == Terrain::Ground,
        }
    }

    /// Movement cost multiplier, as a percentage of the base cost.
    ///
    /// An integer percentage rather than a fraction: pathfinding costs must
    /// stay in exact integer arithmetic so that two peers comparing two routes
    /// never disagree by a rounding bit.
    #[inline]
    fn cost_percent(self, terrain: Terrain) -> u32 {
        if self.can_enter(terrain) {
            100
        } else {
            // Impassable terrain never reaches a cost lookup; the passability
            // check rejects it first.
            u32::MAX
        }
    }
}

/// The world grid.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Map {
    width: u16,
    height: u16,
    terrain: Vec<Terrain>,
    /// Harvestable ore per cell.
    ///
    /// A parallel array rather than a variant of [`Terrain`], because ore is
    /// not a *kind* of ground — it sits on top of ground that units still walk
    /// over, and it changes constantly as harvesters work while the terrain
    /// beneath never does. Folding it into the terrain enum would mean
    /// mutating passability every time a harvester took a load.
    ore: Vec<u16>,
}

impl Map {
    /// Creates a map filled with [`Terrain::Ground`].
    ///
    /// # Panics
    /// If either dimension is zero.
    pub fn new(width: u16, height: u16) -> Map {
        assert!(width > 0 && height > 0, "a map needs a positive size");
        Map {
            width,
            height,
            ore: vec![0; width as usize * height as usize],
            terrain: vec![Terrain::Ground; width as usize * height as usize],
        }
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.width as i32
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.height as i32
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        self.terrain.len()
    }

    #[inline]
    pub fn contains(&self, cell: Cell) -> bool {
        cell.x >= 0 && cell.y >= 0 && cell.x < self.width() && cell.y < self.height()
    }

    /// Flat index of a cell, or `None` if outside the map.
    #[inline]
    pub fn index(&self, cell: Cell) -> Option<u32> {
        if self.contains(cell) {
            Some(cell.y as u32 * self.width as u32 + cell.x as u32)
        } else {
            None
        }
    }

    /// The cell for a flat index. Inverse of [`Map::index`].
    #[inline]
    pub fn cell_at(&self, index: u32) -> Cell {
        debug_assert!((index as usize) < self.terrain.len(), "index out of bounds");
        Cell::new(
            (index % self.width as u32) as i32,
            (index / self.width as u32) as i32,
        )
    }

    /// Terrain at a cell. Cells outside the map read as [`Terrain::Rock`], so
    /// off-map lookups are impassable rather than an error to handle at every
    /// call site.
    #[inline]
    pub fn terrain(&self, cell: Cell) -> Terrain {
        match self.index(cell) {
            Some(i) => self.terrain[i as usize],
            None => Terrain::Rock,
        }
    }

    pub fn set_terrain(&mut self, cell: Cell, terrain: Terrain) {
        if let Some(i) = self.index(cell) {
            self.terrain[i as usize] = terrain;
        }
    }

    /// Fills a rectangle, clipped to the map.
    pub fn fill_rect(&mut self, from: Cell, to: Cell, terrain: Terrain) {
        for y in from.y.min(to.y)..=from.y.max(to.y) {
            for x in from.x.min(to.x)..=from.x.max(to.x) {
                self.set_terrain(Cell::new(x, y), terrain);
            }
        }
    }

    #[inline]
    pub fn is_passable(&self, cell: Cell, locomotor: Locomotor) -> bool {
        self.contains(cell) && locomotor.can_enter(self.terrain(cell))
    }

    /// Whether a diagonal step between two cells is allowed.
    ///
    /// Both shared orthogonal neighbours must be passable. Without this a unit
    /// would slip diagonally through the join between two blocked cells — which
    /// looks like clipping through a wall corner, and lets units reach places
    /// the player can see they should not.
    #[inline]
    pub fn allows_diagonal(&self, from: Cell, to: Cell, locomotor: Locomotor) -> bool {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx == 0 || dy == 0 {
            return true;
        }
        self.is_passable(Cell::new(from.x + dx, from.y), locomotor)
            && self.is_passable(Cell::new(from.x, from.y + dy), locomotor)
    }

    /// Clamps a position to stay just inside the map bounds.
    pub fn clamp_pos(&self, pos: WorldPos) -> WorldPos {
        let max_x = Fx::from_int(self.width()) - Fx::EPSILON;
        let max_y = Fx::from_int(self.height()) - Fx::EPSILON;
        WorldPos {
            x: pos.x.clamp(Fx::ZERO, max_x),
            y: pos.y.clamp(Fx::ZERO, max_y),
        }
    }
}

/// The most ore one cell can hold.
///
/// A cell is a unit of harvesting, not a continuous quantity: a harvester takes
/// a bite and the cell visibly thins out. Capping it keeps a field's total
/// predictable from its area, which is what makes a map's economy readable.
pub const MAX_ORE_PER_CELL: u16 = 500;

impl Map {
    /// Ore remaining in a cell. Out of bounds reads as none.
    #[inline]
    pub fn ore(&self, cell: Cell) -> u16 {
        self.index(cell).map_or(0, |i| self.ore[i as usize])
    }

    /// Whether a cell has anything left to harvest.
    #[inline]
    pub fn has_ore(&self, cell: Cell) -> bool {
        self.ore(cell) > 0
    }

    /// Sets a cell's ore, clamped to [`MAX_ORE_PER_CELL`].
    pub fn set_ore(&mut self, cell: Cell, amount: u16) {
        if let Some(i) = self.index(cell) {
            self.ore[i as usize] = amount.min(MAX_ORE_PER_CELL);
        }
    }

    /// Removes up to `wanted` ore from a cell, returning what was actually
    /// taken.
    ///
    /// Returns the amount rather than assuming the request was met: a
    /// harvester asking for more than remains must be credited only for what
    /// it got, or the map would fund ore it never held.
    pub fn take_ore(&mut self, cell: Cell, wanted: u16) -> u16 {
        let Some(i) = self.index(cell) else {
            return 0;
        };
        let available = self.ore[i as usize];
        let taken = wanted.min(available);
        self.ore[i as usize] = available - taken;
        taken
    }

    /// Ore left on the whole map.
    pub fn total_ore(&self) -> u64 {
        self.ore.iter().map(|&a| a as u64).sum()
    }

    /// Scatters an ore field centred on `centre`.
    ///
    /// Density falls off with distance so a field has a rich middle and thin
    /// edges, which gives harvesters somewhere obvious to start and makes a
    /// contested field worth fighting over at its centre.
    ///
    /// Deterministic given the same arguments — it uses no randomness at all,
    /// so two peers building a map from the same description get the same ore.
    pub fn add_ore_field(&mut self, centre: Cell, radius: i32, richness: u16) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let cell = Cell::new(centre.x + dx, centre.y + dy);
                // Ore only sits on ground. Scattering it into a lake would show
                // a field no harvester could ever reach.
                if self.terrain(cell) != Terrain::Ground {
                    continue;
                }
                let distance = dx.abs().max(dy.abs());
                if distance > radius {
                    continue;
                }
                let falloff = radius + 1 - distance;
                let amount = (richness as i32 * falloff) / (radius + 1);
                let existing = self.ore(cell) as i32;
                self.set_ore(cell, (existing + amount).min(u16::MAX as i32) as u16);
            }
        }
    }
}

impl StateHash for Map {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u16(self.width);
        h.write_u16(self.height);
        for t in &self.terrain {
            h.write_u8(*t as u8);
        }
        // Ore is part of the world state, not scenery: two peers that disagree
        // about how much is left will disagree about credits a minute later.
        for amount in &self.ore {
            h.write_u16(*amount);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terrain each locomotor may enter.
    ///
    /// Written out as a table rather than as assertions scattered through the
    /// movement tests, because this *is* the rule — a naval unit that can drive
    /// onto grass, or a tank that can ford a lake, is a gameplay bug that no
    /// pathfinding test would catch.
    #[test]
    fn locomotors_enter_only_the_terrain_they_should() {
        use Locomotor::*;
        use Terrain::*;

        // (locomotor, ground, water, rock)
        let expected = [
            (Foot, true, false, false),
            (Wheeled, true, false, false),
            (Tracked, true, false, false),
            // Naval units live on water and nowhere else.
            (Ship, false, true, false),
            // Hover crosses both surfaces, which is the whole point of it.
            (Hover, true, true, false),
            // Aircraft ignore terrain entirely, elevation included.
            (Air, true, true, true),
        ];

        for (locomotor, ground, water, rock) in expected {
            assert_eq!(
                locomotor.can_enter(Ground),
                ground,
                "{locomotor:?} on ground"
            );
            assert_eq!(locomotor.can_enter(Water), water, "{locomotor:?} on water");
            assert_eq!(locomotor.can_enter(Rock), rock, "{locomotor:?} on rock");
        }
    }

    #[test]
    fn only_aircraft_cross_high_ground() {
        // Rock stands in for cliffs and elevation until maps carry height.
        // Everything on the surface must be stopped by it; only aircraft pass.
        for locomotor in [
            Locomotor::Foot,
            Locomotor::Wheeled,
            Locomotor::Tracked,
            Locomotor::Ship,
            Locomotor::Hover,
        ] {
            assert!(
                !locomotor.can_enter(Terrain::Rock),
                "{locomotor:?} should not cross high ground"
            );
        }
        assert!(Locomotor::Air.can_enter(Terrain::Rock));
    }

    #[test]
    fn impassable_terrain_costs_more_than_any_route() {
        // The cost lookup should never be reached for impassable terrain, but
        // if it ever is, the answer has to be "never choose this" rather than a
        // number A* might weigh against a detour.
        assert_eq!(Locomotor::Ship.cost_percent(Terrain::Ground), u32::MAX);
        assert_eq!(Locomotor::Tracked.cost_percent(Terrain::Water), u32::MAX);
        assert_eq!(Locomotor::Air.cost_percent(Terrain::Rock), 100);
    }

    #[test]
    fn cell_centre_is_half_a_cell_in() {
        let c = Cell::new(3, 4);
        assert_eq!(
            c.centre(),
            WorldPos::new(Fx::from_frac(7, 2), Fx::from_frac(9, 2))
        );
        assert_eq!(c.centre().cell(), c, "centre must map back to its own cell");
    }

    #[test]
    fn position_to_cell_uses_floor_not_truncation() {
        // The bug this guards: truncation folds -0.5 and 0.5 into cell 0,
        // making the row at the origin twice as wide as every other.
        assert_eq!(
            WorldPos::new(Fx::from_frac(1, 2), Fx::ZERO).cell(),
            Cell::new(0, 0)
        );
        assert_eq!(
            WorldPos::new(Fx::from_frac(-1, 2), Fx::ZERO).cell(),
            Cell::new(-1, 0)
        );
        assert_eq!(WorldPos::new(-Fx::ONE, Fx::ZERO).cell(), Cell::new(-1, 0));
        assert_eq!(
            WorldPos::new(Fx::from_frac(-3, 2), Fx::ZERO).cell(),
            Cell::new(-2, 0)
        );
    }

    #[test]
    fn every_cell_round_trips_through_its_centre() {
        for y in -3..8 {
            for x in -3..8 {
                let c = Cell::new(x, y);
                assert_eq!(c.centre().cell(), c);
            }
        }
    }

    #[test]
    fn index_and_cell_at_are_inverse() {
        let map = Map::new(17, 11);
        for i in 0..map.cell_count() as u32 {
            assert_eq!(map.index(map.cell_at(i)), Some(i));
        }
    }

    #[test]
    fn out_of_bounds_reads_as_impassable_rock() {
        let map = Map::new(4, 4);
        assert_eq!(map.terrain(Cell::new(-1, 0)), Terrain::Rock);
        assert_eq!(map.terrain(Cell::new(4, 0)), Terrain::Rock);
        assert_eq!(map.terrain(Cell::new(0, 4)), Terrain::Rock);
        assert_eq!(map.index(Cell::new(-1, 0)), None);
        assert!(!map.is_passable(Cell::new(-1, 0), Locomotor::Foot));
    }

    #[test]
    fn negative_coordinates_do_not_alias_valid_cells() {
        // A naive `y * width + x` would map (-1, 1) onto index 3 of a 4-wide
        // map, silently reading a real cell for an off-map lookup.
        let map = Map::new(4, 4);
        assert_eq!(map.index(Cell::new(-1, 1)), None);
        assert_eq!(map.index(Cell::new(3, 0)), Some(3));
    }

    #[test]
    fn locomotor_terrain_rules() {
        assert!(Locomotor::Foot.can_enter(Terrain::Ground));
        assert!(!Locomotor::Foot.can_enter(Terrain::Water));
        assert!(!Locomotor::Tracked.can_enter(Terrain::Rock));
        assert!(Locomotor::Air.can_enter(Terrain::Rock));
        assert!(Locomotor::Air.can_enter(Terrain::Water));
    }

    #[test]
    fn diagonal_through_a_corner_join_is_refused() {
        // Two blocked cells meeting at a corner must not be squeezed through.
        let mut map = Map::new(8, 8);
        map.set_terrain(Cell::new(1, 0), Terrain::Rock);
        map.set_terrain(Cell::new(0, 1), Terrain::Rock);
        assert!(!map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), Locomotor::Foot));

        // One side open is still refused — the original did not allow corner
        // cutting either, and allowing it makes units visibly clip walls.
        map.set_terrain(Cell::new(0, 1), Terrain::Ground);
        assert!(!map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), Locomotor::Foot));

        map.set_terrain(Cell::new(1, 0), Terrain::Ground);
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), Locomotor::Foot));
    }

    #[test]
    fn orthogonal_steps_are_never_blocked_by_the_diagonal_rule() {
        let mut map = Map::new(8, 8);
        map.fill_rect(Cell::new(0, 0), Cell::new(7, 7), Terrain::Rock);
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 0), Locomotor::Foot));
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(0, 1), Locomotor::Foot));
    }

    #[test]
    fn neighbour_order_is_fixed() {
        // Pinned deliberately: this order decides which of two equal-cost paths
        // A* finds, so changing it changes unit behaviour and invalidates
        // recorded replays.
        let n = Cell::new(0, 0).neighbours();
        assert_eq!(
            n,
            [
                Cell::new(1, 0),
                Cell::new(1, 1),
                Cell::new(0, 1),
                Cell::new(-1, 1),
                Cell::new(-1, 0),
                Cell::new(-1, -1),
                Cell::new(0, -1),
                Cell::new(1, -1),
            ]
        );
    }

    #[test]
    fn fill_rect_clips_and_normalises_corners() {
        let mut map = Map::new(8, 8);
        // Reversed corners and an overhang past the edge.
        map.fill_rect(Cell::new(6, 6), Cell::new(2, 2), Terrain::Water);
        assert_eq!(map.terrain(Cell::new(4, 4)), Terrain::Water);
        assert_eq!(map.terrain(Cell::new(2, 2)), Terrain::Water);
        assert_eq!(map.terrain(Cell::new(1, 1)), Terrain::Ground);

        map.fill_rect(Cell::new(-5, -5), Cell::new(0, 0), Terrain::Rock);
        assert_eq!(map.terrain(Cell::new(0, 0)), Terrain::Rock);
    }

    #[test]
    fn heading_and_step_are_consistent() {
        let from = Cell::new(2, 2).centre();
        let to = Cell::new(6, 2).centre();
        let heading = from.heading_to(to).unwrap();
        assert_eq!(heading, Angle::ZERO, "due east");

        let moved = from.step(heading, Fx::from_int(4));
        assert_eq!(moved.cell(), to.cell());
    }

    #[test]
    fn heading_to_self_is_undefined() {
        let p = Cell::new(1, 1).centre();
        assert_eq!(p.heading_to(p), None);
    }

    #[test]
    fn clamp_keeps_positions_inside_the_map() {
        let map = Map::new(10, 10);
        let out = WorldPos::new(Fx::from_int(-5), Fx::from_int(50));
        let clamped = map.clamp_pos(out);
        assert!(
            map.contains(clamped.cell()),
            "clamped position must be on the map"
        );
        assert_eq!(clamped.cell(), Cell::new(0, 9));
    }

    #[test]
    fn distance_matches_the_grid() {
        let a = Cell::new(0, 0);
        let b = Cell::new(3, 4);
        assert_eq!(a.chebyshev_to(b), 4);
        assert_eq!(a.manhattan_to(b), 7);
        assert_eq!(a.centre().dist(b.centre()), Fx::from_int(5));
    }
}
