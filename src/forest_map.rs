//! ForestMap — loads forest/wood polygons from the zone server and provides
//! a fast point-in-polygon query for constraining tree spawning.
//!
//! Polygons are stored in zone-local metres (X+ = east, Z+ = south) — the
//! same coordinate space used everywhere else in the sim.
//!
//! After loading, polygons are rasterised into a coarse boolean grid so that
//! `contains(x, z)` is an O(1) array lookup instead of an O(vertices)
//! ray-cast. The grid cell size is 20 m — fine enough that tree placement
//! looks natural, coarse enough to keep the grid tiny (< 200 k cells even
//! for a 6 km² zone).

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};

const GRID_CELL_SIZE: f64 = 20.0; // metres per cell

#[derive(Debug, Deserialize)]
struct RawPolygon {
    points: Vec<[f64; 2]>, // [x, z]
}

/// Pre-rasterised boolean grid built from the raw polygon set.
#[derive(Debug, Clone)]
struct ForestGrid {
    min_x: f64,
    min_z: f64,
    cell_size: f64,
    cols: usize,
    rows: usize,
    /// Row-major: cells[row * cols + col]
    cells: Vec<bool>,
}

impl ForestGrid {
    fn build(polygons: &[Vec<[f64; 2]>], cell_size: f64) -> Self {
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_z = f64::MAX;
        let mut max_z = f64::MIN;

        for poly in polygons {
            for &[x, z] in poly {
                if x < min_x { min_x = x; }
                if x > max_x { max_x = x; }
                if z < min_z { min_z = z; }
                if z > max_z { max_z = z; }
            }
        }

        // At least 1×1 if somehow all coords are identical
        let cols = (((max_x - min_x) / cell_size).ceil() as usize).max(1);
        let rows = (((max_z - min_z) / cell_size).ceil() as usize).max(1);
        let mut cells = vec![false; cols * rows];

        for row in 0..rows {
            for col in 0..cols {
                let cx = min_x + (col as f64 + 0.5) * cell_size;
                let cz = min_z + (row as f64 + 0.5) * cell_size;
                if polygons.iter().any(|poly| point_in_polygon(cx, cz, poly)) {
                    cells[row * cols + col] = true;
                }
            }
        }

        Self { min_x, min_z, cell_size, cols, rows, cells }
    }

    #[inline]
    fn contains(&self, x: f64, z: f64) -> bool {
        if x < self.min_x || z < self.min_z {
            return false;
        }
        let col = ((x - self.min_x) / self.cell_size) as usize;
        let row = ((z - self.min_z) / self.cell_size) as usize;
        if col >= self.cols || row >= self.rows {
            return false;
        }
        self.cells[row * self.cols + col]
    }
}

/// Loaded forest polygon set for a single zone.
#[derive(Debug, Clone)]
pub struct ForestMap {
    polygons: Vec<Vec<[f64; 2]>>,
    grid: Option<ForestGrid>,
}

impl ForestMap {
    /// Empty map — all positions treated as valid (no forest constraint).
    pub fn empty() -> Self {
        Self { polygons: vec![], grid: None }
    }

    /// Fetch forests.json from the zone server and build a ForestMap.
    /// Returns an empty map (all positions allowed) if the file doesn't exist
    /// or the server is unreachable, so missing data degrades gracefully.
    pub async fn fetch(server_url: &str, zone_id: &str) -> Self {
        let url = format!(
            "{}/world/osm/{}/forests.json",
            server_url.trim_end_matches('/'),
            zone_id
        );
        info!("Fetching forest map from {}", url);

        let result: Result<Self> = async {
            let resp = reqwest::get(&url)
                .await
                .with_context(|| format!("GET {url}"))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                warn!("No forests.json for zone {zone_id} — trees may spawn anywhere");
                return Ok(Self { polygons: vec![], grid: None });
            }

            if !resp.status().is_success() {
                anyhow::bail!("Server returned {} for {}", resp.status(), url);
            }

            let raw: Vec<RawPolygon> = resp
                .json()
                .await
                .context("Failed to parse forests.json")?;

            let polygons = raw.into_iter().map(|p| p.points).collect::<Vec<_>>();
            info!("Loaded {} forest polygons for zone {zone_id}", polygons.len());

            let grid = if polygons.is_empty() {
                None
            } else {
                let g = ForestGrid::build(&polygons, GRID_CELL_SIZE);
                info!(
                    cols = g.cols,
                    rows = g.rows,
                    cells = g.cells.len(),
                    "Forest grid rasterised"
                );
                Some(g)
            };

            Ok(Self { polygons, grid })
        }
        .await;

        match result {
            Ok(map) => map,
            Err(e) => {
                warn!("Could not load forest map: {e:#} — tree spawning unconstrained");
                Self { polygons: vec![], grid: None }
            }
        }
    }

    /// Returns true if this map has no polygons (degraded / not loaded).
    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    /// Returns true if (x, z) falls inside any forest polygon, or if the
    /// map is empty (no forest data → allow everywhere).
    ///
    /// O(1) — uses the pre-rasterised grid built at load time.
    #[inline]
    pub fn contains(&self, x: f64, z: f64) -> bool {
        match &self.grid {
            None => true,
            Some(g) => g.contains(x, z),
        }
    }
}

/// Ray-casting point-in-polygon. Polygon points are [x, z] pairs.
/// Used only during grid construction — not on the hot path.
fn point_in_polygon(px: f64, pz: f64, poly: &[[f64; 2]]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = poly[i][0];
        let zi = poly[i][1];
        let xj = poly[j][0];
        let zj = poly[j][1];
        if ((zi > pz) != (zj > pz)) && (px < (xj - xi) * (pz - zi) / (zj - zi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}
