#![allow(dead_code)]
//! Terrain data structures and spatial queries.
//!
//! Provides a 128x128 grid (subdivided from server's 64x64 via bilinear
//! interpolation) containing elevation, walkability, movement cost, biome,
//! and slope data for each cell.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::types::Vector3;

// ============================================================================
// Walkability Flags (matches server NavmeshPipeline)
// ============================================================================

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct WalkabilityFlags: u32 {
        const WALKABLE           = 0;
        const BLOCKED_STRUCTURE  = 1 << 0;
        const BLOCKED_WATER      = 1 << 1;
        const BLOCKED_SLOPE      = 1 << 2;
        const BLOCKED_CORRUPTION = 1 << 3;
        const ROAD               = 1 << 4;
        const DENSE_VEGETATION   = 1 << 5;
        const RUBBLE             = 1 << 6;
        const INDOOR             = 1 << 7;
    }
}

impl WalkabilityFlags {
    /// Mask for any blocking flag.
    pub const BLOCK_MASK: u32 =
        Self::BLOCKED_STRUCTURE.bits()
        | Self::BLOCKED_WATER.bits()
        | Self::BLOCKED_SLOPE.bits()
        | Self::BLOCKED_CORRUPTION.bits();

    pub fn is_blocked(self) -> bool {
        (self.bits() & Self::BLOCK_MASK) != 0
    }

    /// True if the cell is a structure footprint (any kind).
    pub fn is_structure(self) -> bool {
        self.contains(Self::BLOCKED_STRUCTURE) || self.contains(Self::INDOOR)
    }

    /// True if the cell is rubble from collapsed structures.
    pub fn is_rubble(self) -> bool {
        self.contains(Self::RUBBLE)
    }

    /// True if the cell can serve as shelter (rubble without full block).
    pub fn is_shelter(self) -> bool {
        self.is_rubble() && !self.contains(Self::BLOCKED_STRUCTURE)
    }
}

// ============================================================================
// Biome Types (matches server BiomePipeline)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TerrainBiome {
    Forest,
    Scrub,
    Grassland,
    Marsh,
    Desert,
    Rocky,
    Tundra,
    Ruins,
    Water,
    Coastal,
    Farmland,
}

// ============================================================================
// Structure Types (matches server RuinGenPipeline)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StructureCondition {
    Intact,
    Damaged,
    PartialCollapse,
    Collapsed,
    Burned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Structure {
    pub id: String,
    #[serde(rename = "type")]
    pub structure_type: String,
    pub condition: StructureCondition,
    /// Normalized position within tile (0-1).
    pub x: f64,
    pub z: f64,
    pub scale: f64,
    pub floors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Road {
    #[serde(rename = "startX")]
    pub start_x: f64,
    #[serde(rename = "startZ")]
    pub start_z: f64,
    #[serde(rename = "endX")]
    pub end_x: f64,
    #[serde(rename = "endZ")]
    pub end_z: f64,
    pub width: f64,
    #[serde(rename = "isMajor")]
    pub is_major: bool,
    pub condition: StructureCondition,
}

// ============================================================================
// Terrain Grid
// ============================================================================

/// Resolution of the terrain grid (subdivided from server's 64x64).
pub const GRID_RESOLUTION: usize = 128;
pub const GRID_CELLS: usize = GRID_RESOLUTION * GRID_RESOLUTION;

/// A single cell in the terrain grid.
#[derive(Debug, Clone, Copy)]
pub struct TerrainCell {
    pub elevation: f32,
    pub slope: f32,
    pub movement_cost: f32,
    pub walkability: WalkabilityFlags,
    pub biome: TerrainBiome,
}

impl Default for TerrainCell {
    fn default() -> Self {
        Self {
            elevation: 0.0,
            slope: 0.0,
            movement_cost: 1.0,
            walkability: WalkabilityFlags::empty(),
            biome: TerrainBiome::Grassland,
        }
    }
}

/// The main terrain data structure. Holds a 128x128 grid of cells
/// covering a single tile (~2.5km at z=14).
#[derive(Clone)]
pub struct TerrainGrid {
    /// Tile ID (e.g., "14_4823_6127").
    pub tile_id: String,
    /// Cell size in game-world meters.
    pub cell_size: f64,
    /// Total tile size in game-world meters.
    pub tile_size: f64,
    /// World-space origin (min corner) of this tile.
    pub origin: Vector3,
    /// Grid cells, row-major order (GRID_RESOLUTION x GRID_RESOLUTION).
    pub cells: Vec<TerrainCell>,
    /// Structures from the ruin layout.
    pub structures: Vec<Structure>,
    /// Roads from the ruin layout.
    pub roads: Vec<Road>,
}

impl TerrainGrid {
    /// Build a terrain grid from server tile data.
    pub fn from_server_data(
        tile_id: String,
        tile_size: f64,
        origin: Vector3,
        server_navmesh: &ServerNavmesh,
        server_biome: Option<&ServerBiomeData>,
        server_ruins: Option<&ServerRuinLayout>,
    ) -> Self {
        let cell_size = tile_size / GRID_RESOLUTION as f64;

        // Subdivide 64x64 server grid to 128x128 via bilinear interpolation
        let mut cells = vec![TerrainCell::default(); GRID_CELLS];

        for row in 0..GRID_RESOLUTION {
            for col in 0..GRID_RESOLUTION {
                let idx = row * GRID_RESOLUTION + col;

                // Map 128x128 coords to 64x64 source (bilinear interpolation)
                let src_col = col as f64 * (server_navmesh.resolution as f64 - 1.0)
                    / (GRID_RESOLUTION as f64 - 1.0);
                let src_row = row as f64 * (server_navmesh.resolution as f64 - 1.0)
                    / (GRID_RESOLUTION as f64 - 1.0);

                let cell = interpolate_cell(server_navmesh, src_col, src_row);
                cells[idx] = cell;
            }
        }

        // Apply biome distribution if available
        if let Some(biome_data) = server_biome {
            let dominant = parse_biome(&biome_data.dominant_biome);
            for cell in cells.iter_mut() {
                cell.biome = dominant;
            }
        }

        let structures = server_ruins
            .map(|r| r.structures.clone())
            .unwrap_or_default();
        let roads = server_ruins
            .map(|r| r.roads.clone())
            .unwrap_or_default();

        Self {
            tile_id,
            cell_size,
            tile_size,
            origin,
            cells,
            structures,
            roads,
        }
    }

    // ────────────────────────────────────────────────────────────────────────
    // Coordinate Conversion
    // ────────────────────────────────────────────────────────────────────────

    /// Convert world coordinates to grid cell indices.
    /// Returns None if out of bounds.
    pub fn world_to_cell(&self, x: f64, z: f64) -> Option<(usize, usize)> {
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;

        if lx < 0.0 || lz < 0.0 {
            return None;
        }

        let col = (lx / self.cell_size) as usize;
        let row = (lz / self.cell_size) as usize;

        if col >= GRID_RESOLUTION || row >= GRID_RESOLUTION {
            return None;
        }

        Some((col, row))
    }

    /// Convert grid cell indices to world coordinates (center of cell).
    pub fn cell_to_world(&self, col: usize, row: usize) -> Vector3 {
        Vector3 {
            x: self.origin.x + (col as f64 + 0.5) * self.cell_size,
            y: self.get_elevation_at_cell(col, row) as f64,
            z: self.origin.z + (row as f64 + 0.5) * self.cell_size,
        }
    }

    fn get_cell(&self, col: usize, row: usize) -> &TerrainCell {
        &self.cells[row * GRID_RESOLUTION + col]
    }

    // ────────────────────────────────────────────────────────────────────────
    // Spatial Queries
    // ────────────────────────────────────────────────────────────────────────

    /// Check if a world position is walkable.
    pub fn is_walkable(&self, x: f64, z: f64) -> bool {
        match self.world_to_cell(x, z) {
            Some((col, row)) => !self.get_cell(col, row).walkability.is_blocked(),
            None => false,
        }
    }

    /// Get elevation at a world position (meters).
    pub fn get_elevation(&self, x: f64, z: f64) -> f32 {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).elevation,
            None => 0.0,
        }
    }

    fn get_elevation_at_cell(&self, col: usize, row: usize) -> f32 {
        self.get_cell(col, row).elevation
    }

    /// Get movement cost multiplier at a world position.
    pub fn get_movement_cost(&self, x: f64, z: f64) -> f32 {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).movement_cost,
            None => f32::INFINITY,
        }
    }

    /// Get biome at a world position.
    pub fn get_biome_at(&self, x: f64, z: f64) -> TerrainBiome {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).biome,
            None => TerrainBiome::Grassland,
        }
    }

    /// Check if a world position is water.
    pub fn is_water(&self, x: f64, z: f64) -> bool {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self
                .get_cell(col, row)
                .walkability
                .contains(WalkabilityFlags::BLOCKED_WATER),
            None => false,
        }
    }

    /// Check if a world position is a road.
    pub fn is_road(&self, x: f64, z: f64) -> bool {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self
                .get_cell(col, row)
                .walkability
                .contains(WalkabilityFlags::ROAD),
            None => false,
        }
    }

    /// Check if a world position is inside any structure footprint.
    pub fn is_structure(&self, x: f64, z: f64) -> bool {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).walkability.is_structure(),
            None => false,
        }
    }

    /// Check if a world position can serve as shelter.
    pub fn is_shelter(&self, x: f64, z: f64) -> bool {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).walkability.is_shelter(),
            None => false,
        }
    }

    /// Get slope at a world position (degrees).
    pub fn get_slope(&self, x: f64, z: f64) -> f32 {
        match self.world_to_cell(x, z) {
            Some((col, row)) => self.get_cell(col, row).slope,
            None => 90.0,
        }
    }

    /// Find the nearest water cell within max_range (world units).
    /// Returns the world-space center of the nearest water cell.
    pub fn nearest_water(&self, x: f64, z: f64, max_range: f64) -> Option<Vector3> {
        self.nearest_cell_matching(x, z, max_range, |cell| {
            cell.walkability.contains(WalkabilityFlags::BLOCKED_WATER)
        })
    }

    /// Find the nearest shelter cell within max_range.
    pub fn nearest_shelter(&self, x: f64, z: f64, max_range: f64) -> Option<Vector3> {
        self.nearest_cell_matching(x, z, max_range, |cell| cell.walkability.is_shelter())
    }

    /// Find the nearest cell satisfying a predicate, spiral-searching outward.
    fn nearest_cell_matching(
        &self,
        x: f64,
        z: f64,
        max_range: f64,
        predicate: impl Fn(&TerrainCell) -> bool,
    ) -> Option<Vector3> {
        let (center_col, center_row) = self.world_to_cell(x, z)?;
        let max_cells = (max_range / self.cell_size).ceil() as isize;

        let mut best: Option<(f64, Vector3)> = None;

        for dr in -max_cells..=max_cells {
            for dc in -max_cells..=max_cells {
                let col = center_col as isize + dc;
                let row = center_row as isize + dr;

                if col < 0 || row < 0 {
                    continue;
                }

                let col = col as usize;
                let row = row as usize;

                if col >= GRID_RESOLUTION || row >= GRID_RESOLUTION {
                    continue;
                }

                let cell = self.get_cell(col, row);
                if predicate(cell) {
                    let pos = self.cell_to_world(col, row);
                    let dist = ((x - pos.x).powi(2) + (z - pos.z).powi(2)).sqrt();
                    if dist <= max_range {
                        if best.is_none() || dist < best.unwrap().0 {
                            best = Some((dist, pos));
                        }
                    }
                }
            }
        }

        best.map(|(_, pos)| pos)
    }

    /// Check if a position is valid for flora spawning.
    /// No flora inside structure footprints, in water, or on intact roads.
    pub fn can_spawn_flora(&self, x: f64, z: f64, is_ground_cover: bool) -> bool {
        let (col, row) = match self.world_to_cell(x, z) {
            Some(c) => c,
            None => return false,
        };

        let cell = self.get_cell(col, row);

        // No flora in water
        if cell.walkability.contains(WalkabilityFlags::BLOCKED_WATER) {
            return false;
        }

        // No flora inside ANY structure footprint (even rubble)
        if cell.walkability.is_structure() || cell.walkability.is_rubble() {
            return false;
        }

        // No flora on intact roads; ground cover ok on damaged/collapsed roads
        if cell.walkability.contains(WalkabilityFlags::ROAD) {
            return is_ground_cover;
        }

        // Blocked slopes can't support plants
        if cell.walkability.contains(WalkabilityFlags::BLOCKED_SLOPE) {
            return false;
        }

        true
    }

    /// Find a random walkable cell with a specific biome within the grid.
    pub fn find_spawn_position(
        &self,
        biome: TerrainBiome,
        rng: &mut impl rand::Rng,
    ) -> Option<Vector3> {
        use rand::seq::SliceRandom;

        // Collect candidate cells
        let candidates: Vec<(usize, usize)> = (0..GRID_CELLS)
            .filter_map(|idx| {
                let row = idx / GRID_RESOLUTION;
                let col = idx % GRID_RESOLUTION;
                let cell = &self.cells[idx];

                if cell.biome == biome && !cell.walkability.is_blocked() {
                    Some((col, row))
                } else {
                    None
                }
            })
            .collect();

        candidates
            .choose(rng)
            .map(|&(col, row)| self.cell_to_world(col, row))
    }

    /// Get walkability flags at cell coordinates (for pathfinding).
    pub fn get_cell_walkability(&self, col: usize, row: usize) -> WalkabilityFlags {
        if col >= GRID_RESOLUTION || row >= GRID_RESOLUTION {
            return WalkabilityFlags::BLOCKED_STRUCTURE;
        }
        self.cells[row * GRID_RESOLUTION + col].walkability
    }

    /// Get movement cost at cell coordinates (for pathfinding).
    pub fn get_cell_cost(&self, col: usize, row: usize) -> f32 {
        if col >= GRID_RESOLUTION || row >= GRID_RESOLUTION {
            return f32::INFINITY;
        }
        self.cells[row * GRID_RESOLUTION + col].movement_cost
    }

    /// World bounds of this terrain grid.
    pub fn world_bounds(&self) -> (Vector3, Vector3) {
        let min = self.origin;
        let max = Vector3 {
            x: self.origin.x + self.tile_size,
            y: 0.0,
            z: self.origin.z + self.tile_size,
        };
        (min, max)
    }
}

// ============================================================================
// Server Data Types (for JSON deserialization from HTTP API)
// ============================================================================

/// Navmesh cell as received from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerNavCell {
    pub flags: u32,
    pub cost: f32,
    pub elevation: f32,
    pub slope: f32,
}

/// Navmesh as received from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerNavmesh {
    #[serde(rename = "tileId")]
    pub tile_id: String,
    pub resolution: usize,
    #[serde(rename = "cellSize")]
    pub cell_size: f64,
    pub cells: Vec<ServerNavCell>,
}

/// Biome data as received from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerBiomeData {
    #[serde(rename = "tileId")]
    pub tile_id: String,
    #[serde(rename = "dominantBiome")]
    pub dominant_biome: String,
    #[serde(rename = "hasWater")]
    pub has_water: bool,
    #[serde(rename = "temperatureModifier")]
    pub temperature_modifier: f64,
    #[serde(rename = "moistureLevel")]
    pub moisture_level: f64,
}

/// Ruin layout as received from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerRuinLayout {
    #[serde(rename = "tileId")]
    pub tile_id: String,
    pub structures: Vec<Structure>,
    pub roads: Vec<Road>,
    #[serde(rename = "structureCount")]
    pub structure_count: usize,
    #[serde(rename = "averageDamage")]
    pub average_damage: f64,
}

/// Elevation data as received from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerElevationData {
    #[serde(rename = "tileId")]
    pub tile_id: String,
    pub width: usize,
    pub height: usize,
    #[serde(rename = "minElevation")]
    pub min_elevation: f64,
    #[serde(rename = "maxElevation")]
    pub max_elevation: f64,
    pub elevations: Vec<f64>,
}

/// Combined tile data as received from GET /api/tiles/:tileId.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerTileData {
    #[serde(rename = "tileId")]
    pub tile_id: String,
    pub bounds: Option<TileBounds>,
    pub elevation: Option<ServerElevationData>,
    pub navmesh: Option<ServerNavmesh>,
    pub biome: Option<ServerBiomeData>,
    pub ruins: Option<ServerRuinLayout>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TileBounds {
    pub north: f64,
    pub south: f64,
    pub east: f64,
    pub west: f64,
}

/// Tile manifest entry from GET /api/tiles.
#[derive(Debug, Clone, Deserialize)]
pub struct TileManifestEntry {
    pub id: String,
    pub z: u32,
    pub x: u32,
    pub y: u32,
    pub bounds: Option<TileBounds>,
    pub layers: TileLayerAvailability,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TileLayerAvailability {
    pub elevation: bool,
    pub navmesh: bool,
    pub biome: bool,
    pub ruins: bool,
    pub water: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TileManifest {
    pub tiles: Vec<TileManifestEntry>,
    pub count: usize,
}

// ============================================================================
// Interpolation Helpers
// ============================================================================

/// Bilinear interpolation of a server navmesh cell at fractional coordinates.
fn interpolate_cell(navmesh: &ServerNavmesh, src_col: f64, src_row: f64) -> TerrainCell {
    let res = navmesh.resolution;

    let c0 = src_col.floor() as usize;
    let r0 = src_row.floor() as usize;
    let c1 = (c0 + 1).min(res - 1);
    let r1 = (r0 + 1).min(res - 1);

    let fc = src_col - c0 as f64;
    let fr = src_row - r0 as f64;

    // Sample 4 corners
    let tl = &navmesh.cells[r0 * res + c0];
    let tr = &navmesh.cells[r0 * res + c1];
    let bl = &navmesh.cells[r1 * res + c0];
    let br = &navmesh.cells[r1 * res + c1];

    // Interpolate continuous values
    let elevation = bilinear(tl.elevation, tr.elevation, bl.elevation, br.elevation, fc, fr);
    let slope = bilinear(tl.slope, tr.slope, bl.slope, br.slope, fc, fr);
    let movement_cost = bilinear(tl.cost, tr.cost, bl.cost, br.cost, fc, fr);

    // For flags, use nearest-neighbor (discrete values)
    let nearest = if fc < 0.5 {
        if fr < 0.5 { tl } else { bl }
    } else {
        if fr < 0.5 { tr } else { br }
    };

    let walkability = WalkabilityFlags::from_bits_truncate(nearest.flags);

    TerrainCell {
        elevation,
        slope,
        movement_cost,
        walkability,
        biome: TerrainBiome::Grassland, // Will be overridden by biome data
    }
}

fn bilinear(tl: f32, tr: f32, bl: f32, br: f32, fx: f64, fy: f64) -> f32 {
    let top = tl as f64 * (1.0 - fx) + tr as f64 * fx;
    let bot = bl as f64 * (1.0 - fx) + br as f64 * fx;
    (top * (1.0 - fy) + bot * fy) as f32
}

fn parse_biome(s: &str) -> TerrainBiome {
    match s {
        "FOREST" => TerrainBiome::Forest,
        "SCRUB" => TerrainBiome::Scrub,
        "GRASSLAND" => TerrainBiome::Grassland,
        "MARSH" => TerrainBiome::Marsh,
        "DESERT" => TerrainBiome::Desert,
        "ROCKY" => TerrainBiome::Rocky,
        "TUNDRA" => TerrainBiome::Tundra,
        "RUINS" => TerrainBiome::Ruins,
        "WATER" => TerrainBiome::Water,
        "COASTAL" => TerrainBiome::Coastal,
        "FARMLAND" => TerrainBiome::Farmland,
        _ => TerrainBiome::Grassland,
    }
}
