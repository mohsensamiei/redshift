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
pub use redshift_data::traits::{Locomotor, Surface};

/// The largest height difference a ground unit can step across.
///
/// One level is a slope; two is a cliff. The original's maps are built from
/// ramps between adjacent levels, so anything larger is meant to be an
/// obstacle rather than a climb.
pub const MAX_WALKABLE_STEP: u8 = 1;

/// Extra range per level of elevation, as a percentage.
///
/// **Not verified against the original.** The advantage is faithful; the size
/// of it is a guess, flagged in TODO.md with the other unverified rates.
pub const HEIGHT_RANGE_BONUS_PERCENT: u32 = 15;

/// The set of surfaces a unit may cross.
///
/// A bitmask rather than a `Vec<Surface>`: pathfinding reads this millions of
/// times a match and it has to be `Copy`, so a unit's resolved stats stay a
/// plain value type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct SurfaceMask(u8);

impl SurfaceMask {
    /// Goes nowhere. Structures, and anything with no `Mobile` trait.
    pub const NONE: SurfaceMask = SurfaceMask(0);

    const fn bit(surface: Surface) -> u8 {
        match surface {
            Surface::Land => 1,
            Surface::Water => 2,
            Surface::Height => 4,
        }
    }

    pub fn from_surfaces(surfaces: &[Surface]) -> SurfaceMask {
        SurfaceMask(surfaces.iter().fold(0, |acc, s| acc | Self::bit(*s)))
    }

    #[inline]
    pub fn allows(self, surface: Surface) -> bool {
        self.0 & Self::bit(surface) != 0
    }

    #[inline]
    pub fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    pub fn is_immobile(self) -> bool {
        self.0 == 0
    }
}

/// Which surface a cell presents.
///
/// The bridge between the map's terrain and a unit's declared surfaces. It
/// lives here rather than in the data crate because it is about the map, and
/// the data crate must not need to know what a `Terrain` is.
pub fn surface_of(terrain: Terrain) -> Surface {
    match terrain {
        Terrain::Ground => Surface::Land,
        Terrain::Water => Surface::Water,
        // Rock stands in for cliffs and high ground until maps carry real
        // elevation.
        Terrain::Rock => Surface::Height,
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
    /// Ground level per cell.
    ///
    /// A parallel array for the same reason ore is one: elevation is a property
    /// *of* a cell rather than a kind of cell. A unit walks on level 2 ground
    /// exactly as it walks on level 0 — what matters is the difference between
    /// adjacent cells, not the number itself.
    ///
    /// Rock previously stood in for high ground, which kept the movement
    /// restriction and lost everything else: real elevation blocks a *step*
    /// rather than a cell, and gives whoever holds it a longer reach.
    elevation: Vec<u8>,
    /// Cells covered by a building's footprint.
    ///
    /// Kept on the map rather than derived from the units each time a path is
    /// searched. Two reasons: pathfinding asks this question millions of times
    /// and must not walk the entity list to answer it, and putting it here
    /// means `is_passable` accounts for buildings automatically — so no code
    /// that already knows how to avoid a cliff had to learn about buildings.
    blocked: Vec<bool>,
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
            elevation: vec![0; width as usize * height as usize],
            blocked: vec![false; width as usize * height as usize],
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
    pub fn is_passable(&self, cell: Cell, movement: SurfaceMask) -> bool {
        if !self.contains(cell) || !movement.allows(surface_of(self.terrain(cell))) {
            return false;
        }
        // Aircraft fly over buildings; everything on the surface goes round.
        // Answering this here rather than in the pathfinder means every caller
        // that already knew how to avoid a cliff avoids buildings too, with no
        // change at all.
        // Anything that crosses high ground is flying, and flies over buildings
        // too.
        movement.allows(Surface::Height) || !self.is_blocked(cell)
    }

    /// Whether a diagonal step between two cells is allowed.
    ///
    /// Both shared orthogonal neighbours must be passable. Without this a unit
    /// would slip diagonally through the join between two blocked cells — which
    /// looks like clipping through a wall corner, and lets units reach places
    /// the player can see they should not.
    #[inline]
    pub fn allows_diagonal(&self, from: Cell, to: Cell, movement: SurfaceMask) -> bool {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx == 0 || dy == 0 {
            return true;
        }
        self.is_passable(Cell::new(from.x + dx, from.y), movement)
            && self.is_passable(Cell::new(from.x, from.y + dy), movement)
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

impl Map {
    /// Whether a cell is covered by a building.
    #[inline]
    pub fn is_blocked(&self, cell: Cell) -> bool {
        self.index(cell).is_some_and(|i| self.blocked[i as usize])
    }

    /// Marks or clears a rectangle of cells as covered by a building.
    pub fn set_blocked(&mut self, origin: Cell, width: u8, height: u8, blocked: bool) {
        for dy in 0..height as i32 {
            for dx in 0..width as i32 {
                if let Some(i) = self.index(Cell::new(origin.x + dx, origin.y + dy)) {
                    self.blocked[i as usize] = blocked;
                }
            }
        }
    }

    /// Whether a building of this size could stand with its corner at `origin`.
    ///
    /// Every cell has to be on the map, buildable terrain, and free. Checked as
    /// a whole rather than cell by cell at placement time, so a half-placed
    /// building is not a state that can exist.
    pub fn can_place(&self, origin: Cell, width: u8, height: u8) -> bool {
        for dy in 0..height as i32 {
            for dx in 0..width as i32 {
                let cell = Cell::new(origin.x + dx, origin.y + dy);
                if !self.contains(cell) {
                    return false;
                }
                // Buildings need dry, level ground — the same surface a tracked
                // unit can cross.
                if self.terrain(cell) != Terrain::Ground {
                    return false;
                }
                if self.is_blocked(cell) {
                    return false;
                }
                // Ore under a foundation would be unreachable for good.
                if self.has_ore(cell) {
                    return false;
                }
            }
        }
        true
    }
}

impl Map {
    /// The ground level of a cell. Off-map reads as level zero.
    ///
    /// Named `elevation` rather than `height` because `height` is already the
    /// map's second dimension, and two meanings for one word in the same type
    /// is how a bug gets written that reads perfectly.
    #[inline]
    pub fn elevation(&self, cell: Cell) -> u8 {
        self.index(cell)
            .map(|i| self.elevation[i as usize])
            .unwrap_or(0)
    }

    /// Raises or lowers a cell.
    pub fn set_elevation(&mut self, cell: Cell, level: u8) {
        if let Some(i) = self.index(cell) {
            self.elevation[i as usize] = level;
        }
    }

    /// Raises a rectangle of cells to a level.
    pub fn raise_rect(&mut self, from: Cell, to: Cell, level: u8) {
        for y in from.y.min(to.y)..=from.y.max(to.y) {
            for x in from.x.min(to.x)..=from.x.max(to.x) {
                self.set_elevation(Cell::new(x, y), level);
            }
        }
    }

    /// Whether a ground unit can step between two adjacent cells.
    ///
    /// Elevation blocks a *step*, not a cell. That is the whole difference
    /// from the rock that used to stand in for it: high ground is somewhere a
    /// unit can stand and fight, and the cliff is the edge between levels
    /// rather than the plateau itself.
    ///
    /// A step of one level is a slope and is walkable; anything steeper is a
    /// cliff. Flight ignores all of it.
    #[inline]
    pub fn step_is_climbable(&self, from: Cell, to: Cell, movement: SurfaceMask) -> bool {
        if movement.allows(Surface::Height) {
            return true;
        }
        self.elevation(from).abs_diff(self.elevation(to)) <= MAX_WALKABLE_STEP
    }

    /// How much further a unit standing here can see and shoot.
    ///
    /// High ground is worth taking, which it is not if it only stops movement.
    /// Expressed as a percentage so it composes with everything else that
    /// scales a range.
    #[inline]
    pub fn elevation_bonus(&self, cell: Cell) -> u32 {
        100 + self.elevation(cell) as u32 * HEIGHT_RANGE_BONUS_PERCENT
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
        for b in &self.blocked {
            h.write_bool(*b);
        }
        for level in &self.elevation {
            h.write_u8(*level);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary ground-only unit, used throughout these tests.
    const LAND: SurfaceMask = SurfaceMask(1);

    /// The surfaces each movement style crosses **by default**.
    ///
    /// A default, not a rule. Every one of these is overridable per unit, and
    /// the next test is why.
    #[test]
    fn locomotors_default_to_sensible_surfaces() {
        use Locomotor::*;
        let mask = |l: Locomotor| SurfaceMask::from_surfaces(l.default_surfaces());

        for l in [Foot, Wheeled, Tracked] {
            assert!(mask(l).allows(Surface::Land), "{l:?} should walk on land");
            assert!(
                !mask(l).allows(Surface::Water),
                "{l:?} should not swim by default"
            );
            assert!(!mask(l).allows(Surface::Height), "{l:?} should not climb");
        }
        assert!(mask(Ship).allows(Surface::Water));
        assert!(
            !mask(Ship).allows(Surface::Land),
            "a ship does not drive up the beach"
        );
        assert!(mask(Hover).allows(Surface::Land) && mask(Hover).allows(Surface::Water));
        assert!(mask(Air).allows(Surface::Height));
    }

    /// ADR 0006, checked directly.
    ///
    /// Each of these is a unit the original actually has, and each breaks the
    /// rule its own category would imply. All are expressible by listing
    /// surfaces — no new enum variant, no new arm in a match, no engine change.
    #[test]
    fn exceptional_units_need_no_engine_change() {
        use Surface::*;

        // Infantry that swims.
        let amphibious_infantry = SurfaceMask::from_surfaces(&[Land, Water]);
        assert!(amphibious_infantry.allows(Land) && amphibious_infantry.allows(Water));

        // A vehicle that crosses water.
        let hovercraft = SurfaceMask::from_surfaces(&[Land, Water]);
        assert!(hovercraft.allows(Water));

        // Something that crosses high ground without flying. No such unit
        // exists yet, and none would be needed to add one.
        let climber = SurfaceMask::from_surfaces(&[Land, Height]);
        assert!(climber.allows(Height) && !climber.allows(Water));

        // A structure goes nowhere at all.
        assert!(SurfaceMask::NONE.is_immobile());
        assert!(!SurfaceMask::NONE.allows(Land));
    }

    #[test]
    fn a_cell_presents_exactly_one_surface() {
        assert_eq!(surface_of(Terrain::Ground), Surface::Land);
        assert_eq!(surface_of(Terrain::Water), Surface::Water);
        assert_eq!(surface_of(Terrain::Rock), Surface::Height);
    }

    #[test]
    fn passability_asks_the_unit_not_its_category() {
        let mut map = Map::new(16, 16);
        map.set_terrain(Cell::new(4, 4), Terrain::Water);
        map.set_terrain(Cell::new(5, 5), Terrain::Rock);

        let land = SurfaceMask::from_surfaces(&[Surface::Land]);
        let swimmer = SurfaceMask::from_surfaces(&[Surface::Land, Surface::Water]);
        let flier = SurfaceMask::from_surfaces(&[Surface::Land, Surface::Water, Surface::Height]);

        assert!(!map.is_passable(Cell::new(4, 4), land));
        assert!(
            map.is_passable(Cell::new(4, 4), swimmer),
            "a swimmer crosses water"
        );
        assert!(
            !map.is_passable(Cell::new(5, 5), swimmer),
            "but not high ground"
        );
        assert!(map.is_passable(Cell::new(5, 5), flier));
    }

    #[test]
    fn out_of_bounds_reads_as_impassable_rock() {
        let map = Map::new(4, 4);
        assert_eq!(map.terrain(Cell::new(-1, 0)), Terrain::Rock);
        assert_eq!(map.terrain(Cell::new(4, 0)), Terrain::Rock);
        assert_eq!(map.terrain(Cell::new(0, 4)), Terrain::Rock);
        assert_eq!(map.index(Cell::new(-1, 0)), None);
        assert!(!map.is_passable(Cell::new(-1, 0), LAND));
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
    fn diagonal_through_a_corner_join_is_refused() {
        // Two blocked cells meeting at a corner must not be squeezed through.
        let mut map = Map::new(8, 8);
        map.set_terrain(Cell::new(1, 0), Terrain::Rock);
        map.set_terrain(Cell::new(0, 1), Terrain::Rock);
        assert!(!map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), LAND));

        // One side open is still refused — the original did not allow corner
        // cutting either, and allowing it makes units visibly clip walls.
        map.set_terrain(Cell::new(0, 1), Terrain::Ground);
        assert!(!map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), LAND));

        map.set_terrain(Cell::new(1, 0), Terrain::Ground);
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 1), LAND));
    }

    #[test]
    fn orthogonal_steps_are_never_blocked_by_the_diagonal_rule() {
        let mut map = Map::new(8, 8);
        map.fill_rect(Cell::new(0, 0), Cell::new(7, 7), Terrain::Rock);
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(1, 0), LAND));
        assert!(map.allows_diagonal(Cell::new(0, 0), Cell::new(0, 1), LAND));
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
