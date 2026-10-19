//! CivicMap — fetches civic anchor positions from the gateway and provides
//! a tree-exclusion check so no trees spawn on top of libraries, schools,
//! hospitals, etc.
//!
//! Reuses the same `/api/map/anchors` endpoint that the web client uses for
//! beacon placement, so positions are always in sync.

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};

/// Minimum clear radius around any civic anchor (metres).
/// Keeps trees well back from building entrances regardless of ward radius.
const CIVIC_TREE_CLEARANCE: f64 = 40.0;

#[derive(Debug, Deserialize)]
struct AnchorRaw {
    #[serde(rename = "worldX")] x: f64,
    #[serde(rename = "worldZ")] z: f64,
    /// TOWNHALL, LIBRARY, FIRE_STATION, … (gateway loadCivicAnchors).
    #[serde(rename = "type", default)] kind: String,
    /// Beacon ward radius in metres (townhall 500, library 150, …).
    #[serde(rename = "wardRadius", default = "default_ward")] ward: f64,
}

fn default_ward() -> f64 { 100.0 }

/// A beacon as the wildlife sees it: where the Aether is held back.
#[derive(Debug, Clone, Copy)]
pub struct Beacon {
    pub x: f64,
    pub z: f64,
    pub ward: f64,
    pub is_townhall: bool,
}

#[derive(Debug, Deserialize)]
struct AnchorsResponse {
    anchors: Vec<AnchorRaw>,
}

#[derive(Debug, Clone)]
struct CivicAnchor {
    x: f64,
    z: f64,
    kind: String,
    ward: f64,
}

/// Set of civic anchor positions for a zone.
#[derive(Debug, Clone)]
pub struct CivicMap {
    anchors: Vec<CivicAnchor>,
}

impl CivicMap {
    pub fn empty() -> Self {
        Self { anchors: vec![] }
    }

    pub async fn fetch(server_url: &str, zone_id: &str) -> Self {
        let url = format!(
            "{}/api/map/anchors?zoneId={}",
            server_url.trim_end_matches('/'),
            zone_id
        );

        let result: Result<Self> = async {
            let resp = reqwest::get(&url)
                .await
                .with_context(|| format!("GET {url}"))?;

            if !resp.status().is_success() {
                anyhow::bail!("Server returned {} for {}", resp.status(), url);
            }

            let data: AnchorsResponse = resp.json().await.context("Failed to parse anchors")?;
            let anchors = data.anchors.into_iter().map(|a| CivicAnchor { x: a.x, z: a.z, kind: a.kind, ward: a.ward.max(1.0) }).collect::<Vec<_>>();
            info!("Loaded {} civic anchors for zone {zone_id}", anchors.len());
            Ok(Self { anchors })
        }
        .await;

        match result {
            Ok(map) => map,
            Err(e) => {
                warn!("Could not load civic anchors: {e:#} — no civic tree exclusion");
                Self { anchors: vec![] }
            }
        }
    }

    /// Every beacon (civic anchor) with its ward radius.
    pub fn beacons(&self) -> Vec<Beacon> {
        self.anchors
            .iter()
            .map(|a| Beacon { x: a.x, z: a.z, ward: a.ward, is_townhall: a.kind.eq_ignore_ascii_case("TOWNHALL") })
            .collect()
    }

    /// How sheltered from the Aether (x, z) is, 0..1: 1 inside any beacon's
    /// ward, fading linearly to 0 at twice the ward radius. Living things
    /// avoid dense Aether, so prey is weighted toward sheltered ground.
    pub fn shelter(&self, x: f64, z: f64) -> f64 {
        let mut best = 0.0f64;
        for a in &self.anchors {
            let d = ((a.x - x).powi(2) + (a.z - z).powi(2)).sqrt();
            let s = if d <= a.ward { 1.0 } else { (1.0 - (d - a.ward) / a.ward).max(0.0) };
            if s > best { best = s; if best >= 1.0 { break; } }
        }
        best
    }

    /// Returns true if (x, z) is far enough from all civic anchors for a tree to spawn.
    #[inline]
    pub fn clear_for_tree(&self, x: f64, z: f64) -> bool {
        let r2 = CIVIC_TREE_CLEARANCE * CIVIC_TREE_CLEARANCE;
        self.anchors.iter().all(|a| {
            let dx = a.x - x;
            let dz = a.z - z;
            dx * dx + dz * dz > r2
        })
    }
}
