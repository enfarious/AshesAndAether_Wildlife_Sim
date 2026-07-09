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

// ============================================================================
// Fine-resolution building grid
//
// The coarse 128×128 navmesh (~50 m/cell) cannot reliably identify building
// locations for tree-clearance purposes. This grid rasterises OSM building
// polygons at FINE_CELL_SIZE resolution so that `clear_of_structures` is an
// O(1) lookup rather than an O(N_buildings) geometry scan.
//
// Each cell is `true` when its centre is within FINE_CELL_SIZE/2 of a
// building polygon edge *or* inside the polygon. The caller's requested
// clearance radius is handled separately by scanning a cell neighbourhood.
// ============================================================================

const FINE_CELL_SIZE: f64 = 4.0; // metres — fine enough to catch any typical building

#[derive(Clone)]
pub(crate) struct FineGrid {
    min_x: f64,
    min_z: f64,
    cols: usize,
    rows: usize,
    /// Row-major: cells[row * cols + col]. True = building (or near-building) cell.
    cells: Vec<bool>,
}

impl FineGrid {
    fn new_empty(origin_x: f64, origin_z: f64, size: f64) -> Self {
        let cs = FINE_CELL_SIZE;
        let cols = (size / cs).ceil() as usize + 1;
        let rows = (size / cs).ceil() as usize + 1;
        Self { min_x: origin_x, min_z: origin_z, cols, rows, cells: vec![false; cols * rows] }
    }

    /// Mark cells within `clearance` of any building polygon.
    fn rasterize_polygons(&mut self, buildings: &[Vec<[f64; 2]>], clearance: f64) {
        let cs = FINE_CELL_SIZE;
        let cols = self.cols;
        let rows = self.rows;
        let origin_x = self.min_x;
        let origin_z = self.min_z;

        for poly in buildings {
            let mut bmin_x = f64::MAX; let mut bmax_x = f64::MIN;
            let mut bmin_z = f64::MAX; let mut bmax_z = f64::MIN;
            for &[x, z] in poly {
                if x < bmin_x { bmin_x = x; } if x > bmax_x { bmax_x = x; }
                if z < bmin_z { bmin_z = z; } if z > bmax_z { bmax_z = z; }
            }
            let col0 = (((bmin_x - clearance - origin_x) / cs).floor() as isize).clamp(0, cols as isize) as usize;
            let col1 = (((bmax_x + clearance - origin_x) / cs).ceil() as isize + 1).clamp(0, cols as isize) as usize;
            let row0 = (((bmin_z - clearance - origin_z) / cs).floor() as isize).clamp(0, rows as isize) as usize;
            let row1 = (((bmax_z + clearance - origin_z) / cs).ceil() as isize + 1).clamp(0, rows as isize) as usize;

            for row in row0..row1 {
                for col in col0..col1 {
                    if self.cells[row * cols + col] { continue; }
                    let cx = origin_x + (col as f64 + 0.5) * cs;
                    let cz = origin_z + (row as f64 + 0.5) * cs;
                    if _point_to_polygon_dist(cx, cz, poly) <= clearance {
                        self.cells[row * cols + col] = true;
                    }
                }
            }
        }
    }

    /// Mark cells within `clearance` of any road polyline segment.
    fn rasterize_roads(&mut self, roads: &[Vec<[f64; 2]>], clearance: f64) {
        let cs = FINE_CELL_SIZE;
        let cols = self.cols;
        let rows = self.rows;
        let origin_x = self.min_x;
        let origin_z = self.min_z;

        for road in roads {
            for i in 0..road.len().saturating_sub(1) {
                let ax = road[i][0];   let az = road[i][1];
                let bx = road[i+1][0]; let bz = road[i+1][1];
                let bmin_x = ax.min(bx) - clearance;
                let bmax_x = ax.max(bx) + clearance;
                let bmin_z = az.min(bz) - clearance;
                let bmax_z = az.max(bz) + clearance;

                let col0 = (((bmin_x - origin_x) / cs).floor() as isize).clamp(0, cols as isize) as usize;
                let col1 = (((bmax_x - origin_x) / cs).ceil() as isize + 1).clamp(0, cols as isize) as usize;
                let row0 = (((bmin_z - origin_z) / cs).floor() as isize).clamp(0, rows as isize) as usize;
                let row1 = (((bmax_z - origin_z) / cs).ceil() as isize + 1).clamp(0, rows as isize) as usize;

                for row in row0..row1 {
                    for col in col0..col1 {
                        if self.cells[row * cols + col] { continue; }
                        let cx = origin_x + (col as f64 + 0.5) * cs;
                        let cz = origin_z + (row as f64 + 0.5) * cs;
                        if _point_to_segment_dist(cx, cz, ax, az, bx, bz) <= clearance {
                            self.cells[row * cols + col] = true;
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn is_blocked(&self, x: f64, z: f64) -> bool {
        if x < self.min_x || z < self.min_z { return false; }
        let col = ((x - self.min_x) / FINE_CELL_SIZE) as usize;
        let row = ((z - self.min_z) / FINE_CELL_SIZE) as usize;
        if col >= self.cols || row >= self.rows { return false; }
        self.cells[row * self.cols + col]
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
    /// OSM building footprint polygons in world coords [x, z] (zone centre = 0,0).
    pub osm_buildings: Vec<Vec<[f64; 2]>>,
    /// OSM road polylines in world coords [x, z] (zone centre = 0,0).
    pub osm_roads: Vec<Vec<[f64; 2]>>,
    /// OSM closed water polygons (lakes/ponds) in world coords [x, z].
    pub osm_water_polygons: Vec<Vec<[f64; 2]>>,
    /// OSM open waterway polylines (rivers/streams/canals) in world coords [x, z].
    pub osm_waterways: Vec<Vec<[f64; 2]>>,
    /// Fine-resolution building grid for O(1) structure clearance checks
    /// (20 m halo baked in — used for tree placement).
    pub(crate) fine_building_grid: Option<FineGrid>,
    /// Fine-resolution building grid at 0 m clearance — interior-only.
    /// Used by `can_spawn_flora` so non-trees (grass, herbs, vegetables)
    /// don't sprout inside building footprints. The 20 m halo grid is
    /// too aggressive for ground cover; we just want them out of walls.
    pub(crate) fine_building_inside_grid: Option<FineGrid>,
    /// Fine-resolution road grid for O(1) tree-road clearance (10 m halo).
    pub(crate) fine_road_grid: Option<FineGrid>,
    /// Fine-resolution road grid at 3 m — covers the actual driving surface
    /// without pushing flora 10 m back. Used by `can_spawn_flora` so non-
    /// ground-cover plants don't sprout in the middle of small roads the
    /// 50 m navmesh cell can't resolve.
    pub(crate) fine_road_inside_grid: Option<FineGrid>,
    /// Fine-resolution water grid: closed polygons at 0 m PLUS open
    /// waterways at 3 m buffer. Used by both trees (via `clear_of_water`)
    /// and non-trees (via `can_spawn_flora`).
    pub(crate) fine_water_grid: Option<FineGrid>,
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
        server_elevation: Option<&ServerElevationData>,
        server_geometry: Option<&ServerGeometry>,
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

        // Check if navmesh elevation is missing (all zeros) and patch from
        // the dedicated elevation layer or procedural fallback.
        let has_navmesh_elevation = cells.iter().any(|c| c.elevation.abs() > 0.01);

        if !has_navmesh_elevation {
            if let Some(elev) = server_elevation {
                // Use the server's dedicated elevation layer
                tracing::info!(
                    "Navmesh has no elevation — patching from elevation layer ({:.0}-{:.0}m)",
                    elev.min_elevation, elev.max_elevation
                );
                for row in 0..GRID_RESOLUTION {
                    for col in 0..GRID_RESOLUTION {
                        let idx = row * GRID_RESOLUTION + col;
                        // Map 128x128 grid to the elevation layer's resolution
                        let ex = col as f64 / GRID_RESOLUTION as f64 * (elev.width as f64 - 1.0);
                        let ez = row as f64 / GRID_RESOLUTION as f64 * (elev.height as f64 - 1.0);
                        cells[idx].elevation = sample_elevation_grid(elev, ex, ez);
                    }
                }
            } else {
                // No elevation from any source — generate procedural terrain
                // using simple noise so the landscape isn't pancake-flat.
                tracing::warn!(
                    "No elevation data for tile {} — generating procedural elevation",
                    tile_id
                );
                for row in 0..GRID_RESOLUTION {
                    for col in 0..GRID_RESOLUTION {
                        let idx = row * GRID_RESOLUTION + col;
                        let nx = col as f64 / GRID_RESOLUTION as f64;
                        let nz = row as f64 / GRID_RESOLUTION as f64;
                        // Gentle rolling hills (150-350m range, typical for
                        // US northeast / Stephentown NY area)
                        let e1 = pseudo_noise(nx * 2.0, nz * 2.0) * 40.0;
                        let e2 = pseudo_noise(nx * 5.0 + 3.7, nz * 5.0 + 1.3) * 15.0;
                        cells[idx].elevation = (250.0 + e1 + e2) as f32;
                    }
                }
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

        let osm_buildings = server_geometry
            .map(|g| g.buildings.clone())
            .unwrap_or_default();
        let osm_roads = server_geometry
            .map(|g| g.roads.clone())
            .unwrap_or_default();
        let osm_water_polygons = server_geometry
            .map(|g| g.water_polygons.clone())
            .unwrap_or_default();
        let osm_waterways = server_geometry
            .map(|g| g.waterways.clone())
            .unwrap_or_default();

        // Bake fine grids for building / road / water clearance.
        // - buildings:  20 m halo (trees) + 0 m interior-only (all flora)
        // - roads:      10 m halo (trees) +  3 m centerline (non-ground-cover)
        // - water:      0 m polygons (lakes) + 3 m waterway lines (streams)
        // Both groups baked into the same FineGrid where the semantics
        // allow (water: union of polygons + waterways).
        let any_osm = !osm_buildings.is_empty()
            || !osm_roads.is_empty()
            || !osm_water_polygons.is_empty()
            || !osm_waterways.is_empty();
        let (
            fine_building_grid,
            fine_building_inside_grid,
            fine_road_grid,
            fine_road_inside_grid,
            fine_water_grid,
        ) = if !any_osm {
            (None, None, None, None, None)
        } else {
            let mut g  = FineGrid::new_empty(origin.x, origin.z, tile_size);
            let mut gi = FineGrid::new_empty(origin.x, origin.z, tile_size);
            let mut r  = FineGrid::new_empty(origin.x, origin.z, tile_size);
            let mut ri = FineGrid::new_empty(origin.x, origin.z, tile_size);
            let mut w  = FineGrid::new_empty(origin.x, origin.z, tile_size);
            g.rasterize_polygons(&osm_buildings, 20.0);
            gi.rasterize_polygons(&osm_buildings, 0.0);
            r.rasterize_roads(&osm_roads, 10.0);
            ri.rasterize_roads(&osm_roads, 3.0);
            w.rasterize_polygons(&osm_water_polygons, 0.0);
            w.rasterize_roads(&osm_waterways, 3.0);
            (Some(g), Some(gi), Some(r), Some(ri), Some(w))
        };

        Self {
            tile_id,
            cell_size,
            tile_size,
            origin,
            cells,
            structures,
            roads,
            osm_buildings,
            osm_roads,
            osm_water_polygons,
            osm_waterways,
            fine_building_grid,
            fine_building_inside_grid,
            fine_road_grid,
            fine_road_inside_grid,
            fine_water_grid,
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

    /// True if no building falls within `radius` metres of (x, z).
    ///
    /// Uses the fine-resolution building grid (4 m/cell) when OSM data is
    /// available — O(1). Falls back to the coarse navmesh scan otherwise.
    pub fn clear_of_structures(&self, x: f64, z: f64, _radius: f64) -> bool {
        if let Some(grid) = &self.fine_building_grid {
            // Grid was built with the baked 20 m clearance — direct lookup.
            return !grid.is_blocked(x, z);
        }
        // Fallback: coarse navmesh scan (50 m cells — only reliable when no OSM data).
        let half = self.cell_size * 0.5;
        let effective = _radius + half;
        let span = (effective / self.cell_size).ceil() as isize + 1;
        let (cx, cz) = match self.world_to_cell(x, z) {
            Some(c) => c,
            None => return true,
        };
        for dr in -span..=span {
            for dc in -span..=span {
                let col = cx as isize + dc;
                let row = cz as isize + dr;
                if col < 0 || row < 0 || col >= GRID_RESOLUTION as isize || row >= GRID_RESOLUTION as isize {
                    continue;
                }
                let cell = self.get_cell(col as usize, row as usize);
                if cell.walkability.is_structure() || cell.walkability.is_rubble() {
                    let cw = self.cell_to_world(col as usize, row as usize);
                    if ((cw.x - x).powi(2) + (cw.z - z).powi(2)).sqrt() <= effective {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// True if no road falls within `radius` metres of (x, z).
    ///
    /// Uses the fine-resolution road grid (4 m/cell, 10 m clearance baked) when
    /// OSM data is available — O(1). Falls back to coarse navmesh scan otherwise.
    pub fn clear_of_roads(&self, x: f64, z: f64, _radius: f64) -> bool {
        if let Some(grid) = &self.fine_road_grid {
            return !grid.is_blocked(x, z);
        }
        let half = self.cell_size * 0.5;
        let effective = _radius + half;
        let span = (effective / self.cell_size).ceil() as isize + 1;
        let (cx, cz) = match self.world_to_cell(x, z) {
            Some(c) => c,
            None => return true,
        };
        for dr in -span..=span {
            for dc in -span..=span {
                let col = cx as isize + dc;
                let row = cz as isize + dr;
                if col < 0 || row < 0 || col >= GRID_RESOLUTION as isize || row >= GRID_RESOLUTION as isize {
                    continue;
                }
                let cell = self.get_cell(col as usize, row as usize);
                if cell.walkability.contains(WalkabilityFlags::ROAD) {
                    let cw = self.cell_to_world(col as usize, row as usize);
                    if ((cw.x - x).powi(2) + (cw.z - z).powi(2)).sqrt() <= effective {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// True if no water cell falls within `radius` metres of (x, z).
    ///
    /// Consults both the closed-water-polygon fine grid (lakes/ponds —
    /// 4 m precision, when OSM data is loaded) and the navmesh flag scan
    /// (waterways via polyline buffer + slope fallback). Either layer can
    /// veto.
    pub fn clear_of_water(&self, x: f64, z: f64, radius: f64) -> bool {
        if let Some(grid) = &self.fine_water_grid {
            if grid.is_blocked(x, z) { return false; }
        }
        let half = self.cell_size * 0.5;
        let effective = radius + half;
        let span = (effective / self.cell_size).ceil() as isize + 1;
        let (cx, cz) = match self.world_to_cell(x, z) {
            Some(c) => c,
            None => return true,
        };
        for dr in -span..=span {
            for dc in -span..=span {
                let col = cx as isize + dc;
                let row = cz as isize + dr;
                if col < 0 || row < 0 || col >= GRID_RESOLUTION as isize || row >= GRID_RESOLUTION as isize {
                    continue;
                }
                let cell = self.get_cell(col as usize, row as usize);
                if cell.walkability.contains(WalkabilityFlags::BLOCKED_WATER) {
                    let cw = self.cell_to_world(col as usize, row as usize);
                    if ((cw.x - x).powi(2) + (cw.z - z).powi(2)).sqrt() <= effective {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// True if (x, z) is more than `radius` metres from every OSM building polygon.
    /// Returns `true` (no exclusion) when no OSM geometry has been loaded for this tile.
    pub fn clear_of_structure_geometry(&self, x: f64, z: f64, radius: f64) -> bool {
        if self.osm_buildings.is_empty() { return true; }
        for poly in &self.osm_buildings {
            if _point_to_polygon_dist(x, z, poly) <= radius { return false; }
        }
        true
    }

    /// True if (x, z) is more than `radius` metres from every OSM road polyline.
    /// Returns `true` (no exclusion) when no OSM road geometry has been loaded.
    pub fn clear_of_road_geometry(&self, x: f64, z: f64, radius: f64) -> bool {
        if self.osm_roads.is_empty() { return true; }
        for road in &self.osm_roads {
            for i in 0..road.len().saturating_sub(1) {
                let dist = _point_to_segment_dist(x, z, road[i][0], road[i][1], road[i+1][0], road[i+1][1]);
                if dist <= radius { return false; }
            }
        }
        true
    }

    /// No flora inside structure footprints, in water, or on intact roads.
    pub fn can_spawn_flora(&self, x: f64, z: f64, is_ground_cover: bool) -> bool {
        let (col, row) = match self.world_to_cell(x, z) {
            Some(c) => c,
            None => return false,
        };

        let cell = self.get_cell(col, row);

        // No flora in water — check the OSM polygon grid (lakes/ponds) first,
        // then the navmesh flag (waterways + slope fallback).
        if let Some(grid) = &self.fine_water_grid {
            if grid.is_blocked(x, z) { return false; }
        }
        if cell.walkability.contains(WalkabilityFlags::BLOCKED_WATER) {
            return false;
        }

        // No flora inside ANY structure footprint. The 50 m navmesh cell
        // misses small buildings; the 4 m interior-only grid catches them.
        if let Some(grid) = &self.fine_building_inside_grid {
            if grid.is_blocked(x, z) { return false; }
        }
        if cell.walkability.is_structure() || cell.walkability.is_rubble() {
            return false;
        }

        // No flora on intact roads; ground cover ok on damaged/collapsed roads.
        // The 4 m road-interior grid catches small roads the 50 m navmesh
        // cell can't resolve. Ground cover (grass, clover) is still allowed
        // on road surfaces — only tall flora gets pushed off.
        if let Some(grid) = &self.fine_road_inside_grid {
            if grid.is_blocked(x, z) { return is_ground_cover; }
        }
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

/// OSM geometry in world coordinates (zone centre = 0,0) from GET /api/tiles/:tileId.
/// buildings: closed polygons [[x,z]…]; roads: open polylines [[x,z]…];
/// water_polygons: closed lake/pond rings; waterways: open polylines
/// (rivers/streams/canals) — wildlife sim rasterizes each at its own buffer.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerGeometry {
    pub buildings: Vec<Vec<[f64; 2]>>,
    pub roads:     Vec<Vec<[f64; 2]>>,
    #[serde(default, rename = "waterPolygons")]
    pub water_polygons: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pub waterways: Vec<Vec<[f64; 2]>>,
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
    pub geometry: Option<ServerGeometry>,
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
// Geometry Helpers
// ============================================================================

/// Distance from (px, pz) to a polygon: 0.0 if inside, else min edge distance.
fn _point_to_polygon_dist(px: f64, pz: f64, poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    if n < 3 { return f64::MAX; }

    // Ray-casting PIP
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = poly[i][0]; let zi = poly[i][1];
        let xj = poly[j][0]; let zj = poly[j][1];
        if (zi > pz) != (zj > pz) && px < ((xj - xi) * (pz - zi)) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    if inside { return 0.0; }

    // Outside: min distance to any edge
    let mut min_d = f64::MAX;
    j = n - 1;
    for i in 0..n {
        let d = _point_to_segment_dist(px, pz, poly[j][0], poly[j][1], poly[i][0], poly[i][1]);
        if d < min_d { min_d = d; }
        j = i;
    }
    min_d
}

/// Minimum distance from point (px, pz) to line segment (ax,az)→(bx,bz).
fn _point_to_segment_dist(px: f64, pz: f64, ax: f64, az: f64, bx: f64, bz: f64) -> f64 {
    let dx = bx - ax;
    let dz = bz - az;
    let len_sq = dx * dx + dz * dz;
    if len_sq < 1e-10 {
        return ((px - ax).powi(2) + (pz - az).powi(2)).sqrt();
    }
    let t = (((px - ax) * dx + (pz - az) * dz) / len_sq).clamp(0.0, 1.0);
    let cx = ax + t * dx;
    let cz = az + t * dz;
    ((px - cx).powi(2) + (pz - cz).powi(2)).sqrt()
}

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

/// Bilinear sample from the dedicated elevation grid.
fn sample_elevation_grid(elev: &ServerElevationData, ex: f64, ez: f64) -> f32 {
    let c0 = ex.floor() as usize;
    let r0 = ez.floor() as usize;
    let c1 = (c0 + 1).min(elev.width.saturating_sub(1));
    let r1 = (r0 + 1).min(elev.height.saturating_sub(1));

    let fc = ex - c0 as f64;
    let fr = ez - r0 as f64;

    let tl = elev.elevations[r0 * elev.width + c0] as f32;
    let tr = elev.elevations[r0 * elev.width + c1] as f32;
    let bl = elev.elevations[r1 * elev.width + c0] as f32;
    let br = elev.elevations[r1 * elev.width + c1] as f32;

    bilinear(tl, tr, bl, br, fc, fr)
}

/// Simple deterministic noise (same as terrain_client fallback).
fn pseudo_noise(x: f64, y: f64) -> f64 {
    let n = (x * 12.9898 + y * 78.233).sin() * 43758.5453;
    (n - n.floor()) * 2.0 - 1.0
}

fn parse_biome(s: &str) -> TerrainBiome {
    match s {
        "FOREST" | "MIXED_FOREST" | "BOREAL_FOREST" | "TEMPERATE_FOREST" => TerrainBiome::Forest,
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
