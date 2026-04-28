//! HTTP client for fetching terrain data from the game server.
//!
//! Fetches tile data from the server's `/api/tiles` endpoints and
//! builds a `TerrainGrid` for use in the wildlife simulation.

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::terrain::*;
use crate::types::Vector3;

/// Client for fetching terrain data from the game server.
pub struct TerrainClient {
    base_url: String,
    http: reqwest::Client,
}

impl TerrainClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Fetch the tile manifest (list of available tiles).
    pub async fn fetch_manifest(&self) -> Result<TileManifest> {
        let url = format!("{}/api/tiles", self.base_url);
        info!("Fetching tile manifest from {}", url);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to connect to game server")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Server returned {} for tile manifest",
                resp.status()
            );
        }

        let manifest: TileManifest = resp
            .json()
            .await
            .context("Failed to parse tile manifest")?;

        info!("Received manifest with {} tiles", manifest.count);
        Ok(manifest)
    }

    /// Fetch all terrain layers for a single tile.
    pub async fn fetch_tile(&self, tile_id: &str) -> Result<ServerTileData> {
        let url = format!("{}/api/tiles/{}", self.base_url, tile_id);
        info!("Fetching all terrain data for tile {}", tile_id);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch tile {}", tile_id))?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Server returned {} for tile {}",
                resp.status(),
                tile_id
            );
        }

        let data: ServerTileData = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse tile data for {}", tile_id))?;

        info!(
            "Tile {} loaded: navmesh={}, biome={}, ruins={}, elevation={}",
            tile_id,
            data.navmesh.is_some(),
            data.biome.is_some(),
            data.ruins.is_some(),
            data.elevation.is_some(),
        );

        Ok(data)
    }

    /// Fetch tile data and build a TerrainGrid.
    ///
    /// `fallback_tile_size` is the world-space size of the tile in meters, used only
    /// when the server navmesh doesn't provide enough data to compute it.
    /// The actual tile size is derived from `navmesh.resolution * navmesh.cell_size`.
    pub async fn fetch_and_build_grid(
        &self,
        tile_id: &str,
        fallback_tile_size: f64,
        _origin: Vector3,
    ) -> Result<TerrainGrid> {
        let data = self.fetch_tile(tile_id).await?;

        let navmesh = data
            .navmesh
            .as_ref()
            .context("Tile has no navmesh data")?;

        // Derive tile_size from the server's navmesh metadata instead of trusting
        // the config value.  The server's NavmeshPipeline sets:
        //   resolution = 64, cell_size = tileSize / resolution  (~39m for 2500m tiles)
        // so  resolution * cell_size == actual tile extent in world meters.
        let server_tile_size = navmesh.resolution as f64 * navmesh.cell_size;
        let tile_size = if server_tile_size > 1.0 {
            info!(
                "Tile {}: using server-derived tile_size={:.1}m ({}×{:.2}m cells)",
                tile_id, server_tile_size, navmesh.resolution, navmesh.cell_size
            );
            server_tile_size
        } else {
            warn!(
                "Tile {}: server navmesh cell_size looks wrong ({:.4}), using fallback tile_size={:.1}m",
                tile_id, navmesh.cell_size, fallback_tile_size
            );
            fallback_tile_size
        };

        // Center the terrain grid at world origin (the game server places zone
        // origin at 0,0 so -half..+half covers the zone).
        let origin = Vector3::new(-tile_size / 2.0, 0.0, -tile_size / 2.0);

        let grid = TerrainGrid::from_server_data(
            tile_id.to_string(),
            tile_size,
            origin,
            navmesh,
            data.biome.as_ref(),
            data.ruins.as_ref(),
            data.elevation.as_ref(),
            data.geometry.as_ref(),
        );

        let walkable_count = grid
            .cells
            .iter()
            .filter(|c| !c.walkability.is_blocked())
            .count();
        let water_count = grid
            .cells
            .iter()
            .filter(|c| c.walkability.contains(WalkabilityFlags::BLOCKED_WATER))
            .count();
        let structure_count = grid
            .cells
            .iter()
            .filter(|c| c.walkability.is_structure())
            .count();
        let road_count = grid
            .cells
            .iter()
            .filter(|c| c.walkability.contains(WalkabilityFlags::ROAD))
            .count();

        info!(
            "OSM geometry for tile {}: {} buildings, {} roads",
            tile_id,
            grid.osm_buildings.len(),
            grid.osm_roads.len(),
        );
        if !grid.osm_buildings.is_empty() {
            // Log the first building's first vertex to verify coordinate system
            let b = &grid.osm_buildings[0];
            if !b.is_empty() {
                info!("  First building vertex: ({:.1}, {:.1})", b[0][0], b[0][1]);
            }
        }

        let total = GRID_CELLS as f64;
        info!(
            "Built terrain grid for tile {}: {}x{} cells ({:.0}m tile, {:.1}m/cell), \
             {:.1}% walkable, {:.1}% water, {:.1}% structures, {:.1}% roads, \
             elevation {:.0}-{:.0}m",
            tile_id,
            GRID_RESOLUTION,
            GRID_RESOLUTION,
            grid.tile_size,
            grid.cell_size,
            walkable_count as f64 / total * 100.0,
            water_count as f64 / total * 100.0,
            structure_count as f64 / total * 100.0,
            road_count as f64 / total * 100.0,
            grid.cells.iter().map(|c| c.elevation).fold(f32::INFINITY, f32::min),
            grid.cells.iter().map(|c| c.elevation).fold(f32::NEG_INFINITY, f32::max),
        );

        Ok(grid)
    }
}

/// Build a procedural fallback terrain grid for offline mode.
pub fn generate_fallback_terrain(tile_size: f64, origin: Vector3) -> TerrainGrid {
    warn!("Generating fallback procedural terrain (no server connection)");

    let cell_size = tile_size / GRID_RESOLUTION as f64;
    let mut cells = vec![TerrainCell::default(); GRID_CELLS];

    for row in 0..GRID_RESOLUTION {
        for col in 0..GRID_RESOLUTION {
            let idx = row * GRID_RESOLUTION + col;

            // Simple procedural noise
            let nx = col as f64 / GRID_RESOLUTION as f64;
            let nz = row as f64 / GRID_RESOLUTION as f64;

            let elevation = pseudo_noise(nx * 3.0, nz * 3.0) * 50.0 + 100.0;

            // Create some water (low areas near center)
            let dist_from_center =
                ((nx - 0.5).powi(2) + (nz - 0.3).powi(2)).sqrt();
            let is_water = dist_from_center < 0.08;

            // Create some forest patches
            let forest_noise = pseudo_noise(nx * 7.0 + 1.0, nz * 7.0 + 2.0);

            let mut walkability = WalkabilityFlags::empty();
            let mut cost = 1.0f32;
            let biome;

            if is_water {
                walkability |= WalkabilityFlags::BLOCKED_WATER;
                cost = f32::INFINITY;
                biome = TerrainBiome::Water;
            } else if forest_noise > 0.3 {
                walkability |= WalkabilityFlags::DENSE_VEGETATION;
                cost = 1.5;
                biome = TerrainBiome::Forest;
            } else if forest_noise < -0.4 {
                biome = TerrainBiome::Farmland;
            } else {
                biome = TerrainBiome::Grassland;
            }

            cells[idx] = TerrainCell {
                elevation: elevation as f32,
                slope: 0.0,
                movement_cost: cost,
                walkability,
                biome,
            };
        }
    }

    info!(
        "Generated fallback terrain: {}x{} cells, tile_size={}m",
        GRID_RESOLUTION, GRID_RESOLUTION, tile_size
    );

    TerrainGrid {
        tile_id: "fallback".to_string(),
        cell_size,
        tile_size,
        origin,
        cells,
        structures: vec![],
        roads: vec![],
        osm_buildings: vec![],
        osm_roads: vec![],
        fine_building_grid: None,
        fine_road_grid: None,
    }
}

fn pseudo_noise(x: f64, y: f64) -> f64 {
    let n = (x * 12.9898 + y * 78.233).sin() * 43758.5453;
    (n - n.floor()) * 2.0 - 1.0
}
