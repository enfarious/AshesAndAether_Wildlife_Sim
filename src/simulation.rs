#![allow(dead_code)]

//! Core simulation loop for wildlife and flora

use crate::behavior::*;
use crate::climate::Climate;
use crate::climate_subscriber::{ClimateCache, ClimateSnapshot, Season};
use crate::civic_map::CivicMap;
use crate::forest_map::ForestMap;
use crate::pathfinding;
use crate::plant_species::{get_plant_species, PlantType};
use crate::species::get_species;
use crate::terrain::{TerrainBiome, TerrainGrid};
use crate::types::*;
use crate::weather_subscriber::{WeatherCache, WeatherEventType, WeatherSnapshot};
use rand::Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

/// Range in metres within which an entity gets full per-tick simulation.
/// Entities farther than this from every player simulate at a lower rate
/// (~1 Hz instead of 10 Hz) — they still update needs and age, but they don't
/// re-decide behaviour, repath, or move every tick.  Cuts ~90 % of work in
/// typical scenarios where one player can only "see" a few hundred animals.
const ACTIVE_RANGE: f64 = 300.0;
const ACTIVE_RANGE_SQ: f64 = ACTIVE_RANGE * ACTIVE_RANGE;

/// Cell side length for the wildlife spatial grid.  Most species have sight
/// ranges of 50–150 m, so a 100 m cell means perception queries scan a 3×3
/// block of cells.  At 1500 entities in ~36 km² that's ~4 entities per query
/// instead of all 1500 — turns the O(N²) perception scan into ~O(N).
const SPATIAL_CELL_SIZE: f64 = 100.0;

/// A pending pathfinding request.  update_wildlife_entity enqueues these;
/// update() drains them at the end of the tick and runs the A* searches in
/// parallel via rayon.  Paths arrive one tick late (entities use any existing
/// path or fall back to heading-based movement until then).
#[derive(Debug, Clone)]
struct PathRequest {
    entity_id:        String,
    from:             Vector3,
    to:               Vector3,
    requested_at_ms:  i64,
}

const ELDER_AGE_MULTIPLIER: f64 = 3.0;

/// Per-biome tree species table — Vec for stable iteration order + a
/// precomputed total weight so weighted sampling doesn't resum on every pick.
/// Built from the config HashMap once at `set_flora_config` time.
#[derive(Debug, Clone, Default)]
pub struct BiomeTreeTable {
    /// (species_id, weight) pairs. Order is stable across runs.
    pub species: Vec<(String, f64)>,
    /// Sum of all weights — used as the upper bound for the random roll.
    pub total_weight: f64,
}

impl BiomeTreeTable {
    fn from_map(map: &HashMap<String, f64>) -> Self {
        // Keep config order deterministic by sorting species alphabetically.
        let mut species: Vec<(String, f64)> = map
            .iter()
            .filter(|(_, w)| **w > 0.0)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        species.sort_by(|a, b| a.0.cmp(&b.0));
        let total_weight = species.iter().map(|(_, w)| *w).sum();
        Self { species, total_weight }
    }

    /// Weighted-pick a species id. Returns None if the table is empty.
    fn pick<R: Rng>(&self, rng: &mut R) -> Option<&str> {
        if self.species.is_empty() || self.total_weight <= 0.0 {
            return None;
        }
        let mut roll = rng.gen_range(0.0..self.total_weight);
        for (name, weight) in &self.species {
            if roll < *weight {
                return Some(name.as_str());
            }
            roll -= weight;
        }
        self.species.last().map(|(n, _)| n.as_str())
    }
}

/// A single entry in the on-disk flora position cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlant {
    pub species: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub variant: u8,
}
const LEVEL_HEALTH_BONUS: f64 = 0.08;
const LEVEL_DAMAGE_BONUS: f64 = 0.05;

/// Map server-side TerrainBiome to sim-side BiomeType.
fn terrain_biome_to_zone_biome(tb: TerrainBiome) -> BiomeType {
    match tb {
        TerrainBiome::Forest => BiomeType::Forest,
        TerrainBiome::Scrub => BiomeType::Grassland,
        TerrainBiome::Grassland => BiomeType::Grassland,
        TerrainBiome::Marsh => BiomeType::Swamp,
        TerrainBiome::Desert => BiomeType::Desert,
        TerrainBiome::Rocky => BiomeType::Mountain,
        TerrainBiome::Tundra => BiomeType::Tundra,
        TerrainBiome::Ruins => BiomeType::Urban,
        TerrainBiome::Water => BiomeType::Freshwater,
        TerrainBiome::Coastal => BiomeType::Coastal,
        TerrainBiome::Farmland => BiomeType::Grassland,
    }
}

/// Pathfinding state for an entity's current navigation.
struct EntityPath {
    waypoints: Vec<Vector3>,
    current_index: usize,
    target: Vector3,
    computed_at_ms: i64,
}

/// Configuration for spawn rates and limits
pub struct SpawnConfig {
    pub species_id: String,
    pub biomes: Vec<BiomeType>,
    pub spawn_chance: f64,
    pub max_per_zone: usize,
    pub min_distance_from_players: f64,
    pub min_distance_between: f64,
}

/// The main simulation state for a single zone
pub struct ZoneSimulation {
    pub zone_id: String,
    pub biome: BiomeType,
    pub bounds_min: Vector3,
    pub bounds_max: Vector3,

    // Terrain data (from server or procedural fallback)
    pub terrain: Option<TerrainGrid>,

    // Climate system — external (Redis) with internal fallback
    pub climate_cache: Option<ClimateCache>,
    pub current_climate: Option<ClimateSnapshot>,
    /// Internal clock used when the external climate_sim is not running.
    internal_climate: Climate,
    /// Tracks whether we logged the external→internal transition (avoid log spam).
    using_external_climate: bool,

    // Weather system — external (Redis) with calm fallback
    pub weather_cache: Option<WeatherCache>,
    pub current_weather: Option<WeatherSnapshot>,
    /// Tracks whether we logged the external→internal transition.
    using_external_weather: bool,

    // Entities
    pub wildlife: HashMap<String, WildlifeEntity>,
    pub plants: HashMap<String, PlantEntity>,

    // Pathfinding state per entity
    entity_paths: HashMap<String, EntityPath>,

    // External state (from game server)
    pub player_positions: Vec<PlayerPosition>,

    // Forest polygons — restricts tree spawning to OSM wood/forest areas
    pub forest_map: ForestMap,
    // Civic anchor positions — keeps trees clear of beacons/civic buildings
    pub civic_map: CivicMap,

    // Clearance distances for tree spawning (metres from cell edge)
    tree_structure_clearance: f64,
    tree_road_clearance: f64,
    tree_water_clearance: f64,
    /// Trees per km² scattered zone-wide (pass 1).
    tree_zone_density_per_km2: f64,
    /// Additional trees per km² inside forest polygons (pass 2).
    tree_forest_density_per_km2: f64,
    /// Per-biome species composition. At each candidate position, the biome
    /// at (x,z) is looked up and a species is weighted-sampled from this
    /// biome's table. Biomes with no entry produce no trees at those cells.
    /// Stored as a Vec for stable iteration + precomputed total weight so
    /// the spawn hot path doesn't re-sum on every pick.
    tree_species_by_biome: HashMap<BiomeType, BiomeTreeTable>,

    // Water sources (fallback when no terrain)
    pub water_sources: Vec<Vector3>,

    // Timing
    last_update_ms: i64,
    last_spawn_check_ms: i64,
    last_plant_spawn_ms: i64,
    last_plant_update_ms: i64,
    last_status_log_ms: i64,

    // ID generation
    next_entity_id: u64,
    next_plant_id: u64,

    // Pending events to send
    pub pending_events: Vec<WildlifeEvent>,

    // Pathfinding requests collected during the current tick — drained and
    // processed in parallel at end-of-tick.
    pending_path_requests: Vec<PathRequest>,

    // Spatial index for live wildlife — rebuilt at the start of every tick.
    // Maps cell (x, z) coordinates to entity IDs.
    spatial_grid: HashMap<(i32, i32), Vec<String>>,

    // Spatial index for plants.  Plants rarely change so we mark this dirty
    // when plants are added/removed/eaten and only rebuild when needed.
    plant_grid: HashMap<(i32, i32), Vec<String>>,
    plant_grid_dirty: bool,
}

impl ZoneSimulation {
    pub fn new(zone_id: String, biome: BiomeType, bounds_min: Vector3, bounds_max: Vector3) -> Self {
        Self {
            zone_id,
            biome,
            bounds_min,
            bounds_max,
            terrain: None,
            climate_cache: None,
            current_climate: None,
            internal_climate: Climate::default(),
            using_external_climate: false,
            weather_cache: None,
            current_weather: None,
            using_external_weather: false,

            wildlife: HashMap::new(),
            plants: HashMap::new(),
            entity_paths: HashMap::new(),
            player_positions: Vec::new(),
            forest_map: ForestMap::empty(),
            civic_map: CivicMap::empty(),
            tree_structure_clearance: 20.0,
            tree_road_clearance: 10.0,
            tree_water_clearance: 5.0,
            tree_zone_density_per_km2:   135.0,
            tree_forest_density_per_km2: 450.0,
            tree_species_by_biome: HashMap::new(),
            water_sources: Vec::new(),

            last_update_ms: 0,
            last_spawn_check_ms: 0,
            last_plant_spawn_ms: 0,
            last_plant_update_ms: 0,
            last_status_log_ms: 0,
            next_entity_id: 1,
            next_plant_id: 1,
            pending_events: Vec::new(),
            pending_path_requests: Vec::new(),
            spatial_grid: HashMap::new(),
            plant_grid: HashMap::new(),
            plant_grid_dirty: true,
        }
    }

    /// Rebuild the plant spatial grid.  Only includes edible plants —
    /// trees (pine/oak/maple) make up ~95 % of plants but no animal eats
    /// them, so excluding them shrinks the grid 20× and makes herbivore
    /// food perception nearly free.
    fn rebuild_plant_grid(&mut self) {
        self.plant_grid.clear();
        for (id, plant) in &self.plants {
            if !plant.is_alive { continue; }
            let species = match get_plant_species(&plant.species_id) {
                Some(s) => s,
                None => continue,
            };
            // Skip large trees — herbivores don't forage them
            if matches!(species.plant_type, PlantType::Tree) { continue; }
            let cx = (plant.position.x / SPATIAL_CELL_SIZE).floor() as i32;
            let cz = (plant.position.z / SPATIAL_CELL_SIZE).floor() as i32;
            self.plant_grid.entry((cx, cz)).or_default().push(id.clone());
        }
        self.plant_grid_dirty = false;
    }

    fn plant_spatial_query<F: FnMut(&str)>(&self, pos: Vector3, range: f64, mut f: F) {
        let cx = (pos.x / SPATIAL_CELL_SIZE).floor() as i32;
        let cz = (pos.z / SPATIAL_CELL_SIZE).floor() as i32;
        let r  = (range / SPATIAL_CELL_SIZE).ceil() as i32;
        for dx in -r..=r {
            for dz in -r..=r {
                if let Some(ids) = self.plant_grid.get(&(cx + dx, cz + dz)) {
                    for id in ids { f(id); }
                }
            }
        }
    }

    /// Rebuild the spatial grid from current wildlife positions.  Called once
    /// per tick before the entity update loop so build_context() can do
    /// neighbour queries in near-constant time.
    fn rebuild_spatial_grid(&mut self) {
        self.spatial_grid.clear();
        for (id, entity) in &self.wildlife {
            if !entity.is_alive { continue; }
            let cx = (entity.position.x / SPATIAL_CELL_SIZE).floor() as i32;
            let cz = (entity.position.z / SPATIAL_CELL_SIZE).floor() as i32;
            self.spatial_grid.entry((cx, cz)).or_default().push(id.clone());
        }
    }

    /// Iterate all wildlife IDs in cells overlapping a circle of `range`
    /// metres around `pos`.  Caller must still distance-check returned IDs
    /// since cell coverage is rectangular and approximate.
    fn spatial_query<F: FnMut(&str)>(&self, pos: Vector3, range: f64, mut f: F) {
        let cx = (pos.x / SPATIAL_CELL_SIZE).floor() as i32;
        let cz = (pos.z / SPATIAL_CELL_SIZE).floor() as i32;
        let r  = (range / SPATIAL_CELL_SIZE).ceil() as i32;
        for dx in -r..=r {
            for dz in -r..=r {
                if let Some(ids) = self.spatial_grid.get(&(cx + dx, cz + dz)) {
                    for id in ids { f(id); }
                }
            }
        }
    }

    /// Zone area in km².  Used to scale population densities.
    pub fn zone_area_km2(&self) -> f64 {
        let width = (self.bounds_max.x - self.bounds_min.x).abs();
        let depth = (self.bounds_max.z - self.bounds_min.z).abs();
        (width * depth) / 1_000_000.0
    }

    pub fn set_civic_map(&mut self, map: CivicMap) {
        self.civic_map = map;
    }

    /// Set forest polygon map for ecologically accurate tree placement.
    pub fn set_forest_map(&mut self, map: ForestMap) {
        self.forest_map = map;
    }

    /// Configure clearance distances, per-pass densities, and the per-biome
    /// species composition table from the parsed FloraConfig.
    pub fn set_flora_config(&mut self, cfg: &crate::FloraConfig) {
        self.tree_structure_clearance = cfg.tree_building_clearance;
        self.tree_road_clearance      = cfg.tree_road_clearance;
        self.tree_water_clearance     = cfg.tree_water_clearance;
        self.tree_zone_density_per_km2   = cfg.zone_density_per_km2;
        self.tree_forest_density_per_km2 = cfg.forest_density_per_km2;

        // Compile the config HashMap into BiomeTreeTable structs (sorted +
        // total weight cached). Drop biomes whose table has no positive weights.
        self.tree_species_by_biome.clear();
        for (biome, species_map) in &cfg.species_by_biome {
            let table = BiomeTreeTable::from_map(species_map);
            if !table.species.is_empty() {
                self.tree_species_by_biome.insert(*biome, table);
            }
        }

        // Validate species ids against the registry; warn loudly on misses
        // since silent zero-trees-from-this-biome would be hard to debug.
        let mut bad: Vec<String> = Vec::new();
        for table in self.tree_species_by_biome.values() {
            for (sp, _) in &table.species {
                if get_plant_species(sp).is_none() {
                    bad.push(sp.clone());
                }
            }
        }
        if !bad.is_empty() {
            bad.sort();
            bad.dedup();
            tracing::warn!(
                "FloraConfig references unknown species: {:?} — they will be picked but spawn nothing",
                bad
            );
        }
    }

    /// Set terrain data and update bounds to match.
    pub fn set_terrain(&mut self, terrain: TerrainGrid) {
        let (bounds_min, bounds_max) = terrain.world_bounds();
        self.bounds_min = bounds_min;
        self.bounds_max = bounds_max;
        self.terrain = Some(terrain);
    }

    /// Get current time_of_day (for compatibility)
    pub fn time_of_day(&self) -> f64 {
        self.current_climate
            .as_ref()
            .map(|c| c.time_of_day)
            .unwrap_or(12.0)
    }

    /// Initialize climate subscription (call once at startup).
    /// Subscribes to the external climate_sim via Redis. If external data
    /// arrives, it takes priority. Otherwise the internal Climate clock
    /// drives climate state as a fallback.
    pub async fn init_climate(&mut self, redis_url: &str) -> anyhow::Result<()> {
        let cache = ClimateCache::new(self.zone_id.clone());
        cache.start_subscription(redis_url).await?;

        // Wait briefly for external data (max 5 seconds)
        self.current_climate = cache.get_or_wait(5000).await;

        if let Some(ref snapshot) = self.current_climate {
            // Sync internal clock to match external sim's time
            self.internal_climate.day_of_year = snapshot.day_of_year;
            self.internal_climate.time_of_day = snapshot.time_of_day;
            self.internal_climate.year = snapshot.year;
            self.using_external_climate = true;
            info!(
                zone = %self.zone_id,
                "Using external climate_sim (Day {} {:?})",
                snapshot.day_of_year, snapshot.season
            );
        } else {
            // No external data — start with internal fallback immediately
            self.current_climate = Some(self.internal_climate.to_snapshot(&self.zone_id));
            info!(
                zone = %self.zone_id,
                "No external climate_sim detected — using internal fallback"
            );
        }

        self.climate_cache = Some(cache);
        Ok(())
    }

    /// Initialize weather subscription (call once at startup).
    /// Subscribes to the external weather_sim via Redis. Falls back to
    /// calm weather if no external data is available.
    pub async fn init_weather(&mut self, redis_url: &str) -> anyhow::Result<()> {
        let mut cache = WeatherCache::new(self.zone_id.clone());
        cache.start_subscription(redis_url).await?;
        self.current_weather = cache.get().await;

        if self.current_weather.is_some() {
            self.using_external_weather = true;
            info!(zone = %self.zone_id, "Using external weather_sim");
        } else {
            info!(zone = %self.zone_id, "No external weather_sim detected — using calm weather fallback");
        }

        self.weather_cache = Some(cache);
        Ok(())
    }

    /// Set the time scale for the internal fallback climate clock.
    pub fn set_time_scale(&mut self, time_scale: f64) {
        self.internal_climate.time_scale = time_scale;
    }

    /// Main update tick
    pub async fn update(&mut self, now_ms: i64, delta_seconds: f64) {
        let mut rng = rand::thread_rng();

        // ── Climate: prefer external, fall back to internal ──────────
        let external_climate_active = self.climate_cache
            .as_ref()
            .map(|c| c.is_external_active())
            .unwrap_or(false);

        if external_climate_active {
            // External climate_sim is running — use its data
            if let Some(ref cache) = self.climate_cache {
                if let Some(snapshot) = cache.get().await {
                    // Sync internal clock to stay in step (for seamless handoff)
                    self.internal_climate.day_of_year = snapshot.day_of_year;
                    self.internal_climate.time_of_day = snapshot.time_of_day;
                    self.internal_climate.year = snapshot.year;
                    self.current_climate = Some(snapshot);
                }
            }
            if !self.using_external_climate {
                info!(zone = %self.zone_id, "External climate_sim detected — switching from internal fallback");
                self.using_external_climate = true;
            }
        } else {
            // No external data (or stale) — drive climate internally
            self.internal_climate.advance(delta_seconds);
            self.current_climate = Some(self.internal_climate.to_snapshot(&self.zone_id));
            if self.using_external_climate {
                info!(zone = %self.zone_id, "External climate_sim went away — switching to internal fallback");
                self.using_external_climate = false;
            }
        }

        // ── Weather: prefer external, fall back to calm ──────────────
        let external_weather_active = self.weather_cache
            .as_ref()
            .map(|c| c.is_external_active())
            .unwrap_or(false);

        if external_weather_active {
            if let Some(ref cache) = self.weather_cache {
                if let Some(snapshot) = cache.get().await {
                    self.current_weather = Some(snapshot);
                }
            }
            if !self.using_external_weather {
                info!(zone = %self.zone_id, "External weather_sim detected — using external weather");
                self.using_external_weather = true;
            }
        } else {
            // No external weather — use calm defaults
            self.current_weather = Some(WeatherSnapshot {
                zone_id: self.zone_id.clone(),
                timestamp_ms: now_ms,
                active_events: Vec::new(),
                base_wind_speed: 2.0,
                base_wind_direction: 0.0,
                precipitation: 0.0,
                cloud_cover: 0.3,
                visibility: 1000.0,
            });
            if self.using_external_weather {
                info!(zone = %self.zone_id, "External weather_sim went away — using calm weather fallback");
                self.using_external_weather = false;
            }
        }

        // ── PROFILE TIMERS ──
        let _t_start = std::time::Instant::now();
        let _t_grid_start = std::time::Instant::now();

        // Build the spatial index used by perception queries this tick.
        self.rebuild_spatial_grid();
        if self.plant_grid_dirty {
            self.rebuild_plant_grid();
        }
        let t_grid_ms = _t_grid_start.elapsed().as_millis();

        let _t_wildlife_start = std::time::Instant::now();
        let mut near_count = 0;
        let mut coarse_count = 0;

        // AI LOD: full per-tick simulation only for entities near a player;
        // distant entities update at ~1 Hz to keep needs/aging consistent
        // without burning CPU on animals nobody can see.
        let entity_ids: Vec<String> = self.wildlife.keys().cloned().collect();
        for id in entity_ids {
            let (entity_pos, last_tick) = match self.wildlife.get(&id) {
                Some(e) if e.is_alive => (e.position, e.last_update_tick_ms),
                _ => continue,
            };

            let near_player = self.player_positions.iter().any(|p| {
                let dx = entity_pos.x - p.position.x;
                let dz = entity_pos.z - p.position.z;
                dx * dx + dz * dz <= ACTIVE_RANGE_SQ
            });

            if near_player {
                near_count += 1;
                if let Some(e) = self.wildlife.get_mut(&id) {
                    e.last_update_tick_ms = now_ms;
                }
                self.update_wildlife_entity(&id, now_ms, delta_seconds, &mut rng);
            } else if now_ms - last_tick >= 1_000 {
                coarse_count += 1;
                // Far from players — coarse update.  Cap dt at the same value
                // the outer tick uses so a slow tick (or first-ever update
                // with last_tick == 0 against now_ms in the billions) can't
                // pass a giant dt that makes movement code request a path
                // hundreds of metres away.  Far entities effectively run in
                // mild slow-motion, which nobody can see.
                let coarse_dt = if last_tick == 0 {
                    delta_seconds
                } else {
                    let raw = (now_ms - last_tick) as f64 / 1000.0;
                    raw.min(delta_seconds.max(0.25))
                };
                if let Some(e) = self.wildlife.get_mut(&id) {
                    e.last_update_tick_ms = now_ms;
                }
                self.update_wildlife_entity(&id, now_ms, coarse_dt, &mut rng);
            }
        }
        let t_wildlife_ms = _t_wildlife_start.elapsed().as_millis();
        let path_request_count = self.pending_path_requests.len();
        let _t_paths_start = std::time::Instant::now();

        // ── Parallel pathfinding ──
        // update_wildlife_entity above only collects PathRequests; the actual
        // A* searches happen here, in parallel across rayon's thread pool.
        // For ~1500 entities this drops pathfinding from ~600 ms/tick (single
        // core) to under 100 ms on a typical 4–8 core CPU.
        if !self.pending_path_requests.is_empty() {
            if let Some(ref terrain) = self.terrain {
                let requests = std::mem::take(&mut self.pending_path_requests);
                let new_paths: Vec<(String, Option<EntityPath>)> = requests
                    .par_iter()
                    .map(|req| {
                        let path = pathfinding::find_path(terrain, req.from, req.to)
                            .filter(|wpts| !wpts.is_empty())
                            .map(|waypoints| EntityPath {
                                waypoints,
                                current_index:  0,
                                target:         req.to,
                                computed_at_ms: req.requested_at_ms,
                            });
                        (req.entity_id.clone(), path)
                    })
                    .collect();
                for (id, path) in new_paths {
                    match path {
                        Some(p) => { self.entity_paths.insert(id, p); }
                        None    => { self.entity_paths.remove(&id); }
                    }
                }
            } else {
                self.pending_path_requests.clear();
            }
        }
        let t_paths_ms = _t_paths_start.elapsed().as_millis();
        let _t_plants_start = std::time::Instant::now();
        let mut did_plants = false;

        // Update plants at most once per second — stage transitions take hours,
        // so 10 Hz updates burn ~1 sec/tick on 100k+ plants for no visible gain.
        // Pass the real elapsed time so growth math stays correct.
        if now_ms - self.last_plant_update_ms >= 1000 {
            let plant_dt = if self.last_plant_update_ms == 0 {
                delta_seconds
            } else {
                (now_ms - self.last_plant_update_ms) as f64 / 1000.0
            };
            self.last_plant_update_ms = now_ms;
            did_plants = true;

            let plant_ids: Vec<String> = self.plants.keys().cloned().collect();
            for id in plant_ids {
                self.update_plant(&id, plant_dt);
            }
        }
        let t_plants_ms = _t_plants_start.elapsed().as_millis();
        let _t_hunt_start = std::time::Instant::now();

        // Process attacks and interactions
        self.process_hunting(now_ms);
        let t_hunt_ms = _t_hunt_start.elapsed().as_millis();
        let total_ms = _t_start.elapsed().as_millis();
        if total_ms > 200 {
            tracing::warn!(
                "PROFILE total={} grid={} wildlife={} (near={} coarse={}) paths={} (n={}) plants={} (ran={}) hunt={}",
                total_ms, t_grid_ms, t_wildlife_ms, near_count, coarse_count,
                t_paths_ms, path_request_count, t_plants_ms, did_plants, t_hunt_ms,
            );
        }

        // Check wildlife spawns periodically (every 30 seconds)
        if now_ms - self.last_spawn_check_ms > 30_000 {
            self.check_spawns(now_ms, &mut rng);
            self.last_spawn_check_ms = now_ms;
        }

        // Check plant spawns periodically (every 60 seconds)
        if now_ms - self.last_plant_spawn_ms > 60_000 {
            self.check_plant_spawns(now_ms, &mut rng);
            self.last_plant_spawn_ms = now_ms;
        }

        // Log status report periodically (every 30 seconds)
        if now_ms - self.last_status_log_ms > 30_000 {
            self.log_status();
            self.last_status_log_ms = now_ms;
        }

        self.last_update_ms = now_ms;
    }

    /// Log a periodic status summary to the console.
    fn log_status(&self) {
        // ── Wildlife by species and behavior ──
        let mut species_states: HashMap<&str, HashMap<BehaviorState, usize>> = HashMap::new();
        let mut species_totals: HashMap<&str, usize> = HashMap::new();

        for entity in self.wildlife.values() {
            if !entity.is_alive {
                continue;
            }
            let species = entity.species_id.as_str();
            *species_totals.entry(species).or_insert(0) += 1;
            *species_states
                .entry(species)
                .or_default()
                .entry(entity.current_behavior)
                .or_insert(0) += 1;
        }

        // ── Plants by species ──
        let mut plant_counts: HashMap<&str, usize> = HashMap::new();
        let mut plants_alive = 0usize;
        for plant in self.plants.values() {
            if plant.is_alive {
                *plant_counts.entry(plant.species_id.as_str()).or_insert(0) += 1;
                plants_alive += 1;
            }
        }

        // ── Build the log string ──
        let total_wildlife: usize = species_totals.values().sum();

        let mut lines = Vec::new();
        lines.push(format!(
            "──── Zone {} Status ────────────────────────────",
            self.zone_id
        ));
        lines.push(format!("Wildlife alive: {}", total_wildlife));

        // Sort species alphabetically for stable output
        let mut sorted_species: Vec<&&str> = species_totals.keys().collect();
        sorted_species.sort();

        for species in sorted_species {
            let total = species_totals[*species];
            let states = &species_states[*species];

            // Collect non-zero states, sorted by count descending
            let mut state_list: Vec<(BehaviorState, usize)> =
                states.iter().map(|(&s, &c)| (s, c)).collect();
            state_list.sort_by(|a, b| b.1.cmp(&a.1));

            let state_str: Vec<String> = state_list
                .iter()
                .map(|(s, c)| format!("{:?}={}", s, c))
                .collect();

            lines.push(format!(
                "  {:>8}: {:>3}  [{}]",
                species,
                total,
                state_str.join(", ")
            ));
        }

        // Plant summary
        lines.push(format!("Plants alive: {}", plants_alive));
        let mut sorted_plants: Vec<&&str> = plant_counts.keys().collect();
        sorted_plants.sort();
        for species in sorted_plants {
            lines.push(format!("  {:>12}: {}", species, plant_counts[*species]));
        }

        // Climate info if available
        if let Some(ref climate) = self.current_climate {
            lines.push(format!(
                "Climate: {:?} day={} t={:.1}h temp={:.2} growth={:.2}{}",
                climate.season,
                climate.day_of_year,
                climate.time_of_day,
                climate.temperature,
                climate.growth_rate,
                if climate.is_night { " (night)" } else { "" },
            ));
        }

        // Weather info if available
        if let Some(ref weather) = self.current_weather {
            lines.push(format!(
                "Weather: precip={:.2} cloud={:.2} vis={:.2} wind={:.1}m/s events={}",
                weather.precipitation,
                weather.cloud_cover,
                weather.visibility,
                weather.base_wind_speed,
                weather.active_events.len(),
            ));
        }

        lines.push("─────────────────────────────────────────────────".to_string());

        println!("\n{}", lines.join("\n"));
    }

    fn update_wildlife_entity(
        &mut self,
        entity_id: &str,
        now_ms: i64,
        delta_seconds: f64,
        rng: &mut impl Rng,
    ) {
        let _t_entity_start = std::time::Instant::now();
        // First, gather immutable data we need
        let (species_id, is_alive, should_check_birth) = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            (
                entity.species_id.clone(),
                entity.is_alive,
                entity.is_pregnant && entity.pregnancy_ends_at.map_or(false, |t| now_ms >= t),
            )
        };

        if !is_alive {
            return;
        }

        let species = match get_species(&species_id) {
            Some(s) => s,
            None => return,
        };

        // Update needs (inline to avoid borrow issues)
        {
            let entity = self.wildlife.get_mut(entity_id).unwrap();
            let rates = &species.need_decay_rates;

            // Hibernating entities don't consume food/water and recover energy
            if entity.current_behavior == BehaviorState::Hibernating {
                // Slight energy recovery during hibernation
                entity.needs.energy = (entity.needs.energy + 0.1 * delta_seconds).min(100.0);
                // Hunger/thirst decay slowly
                entity.needs.hunger = (entity.needs.hunger - rates.hunger * 0.1 * delta_seconds).max(0.0);
                entity.needs.thirst = (entity.needs.thirst - rates.thirst * 0.1 * delta_seconds).max(0.0);
            } else {
                // Calculate temperature stress multipliers
                let temperature = self.current_climate.as_ref().map(|c| c.temperature).unwrap_or(0.0);
                let hunger_mult = if temperature < -0.3 {
                    1.5 // Cold stress increases hunger
                } else if temperature > 0.6 {
                    1.3 // Heat stress increases thirst more than hunger
                } else {
                    1.0
                };

                let thirst_mult = if temperature > 0.6 {
                    1.5 // Heat stress increases thirst
                } else if temperature < -0.3 {
                    1.2
                } else {
                    1.0
                };

                entity.needs.hunger = (entity.needs.hunger - rates.hunger * hunger_mult * delta_seconds).max(0.0);
                entity.needs.thirst = (entity.needs.thirst - rates.thirst * thirst_mult * delta_seconds).max(0.0);

                let energy_drain = match entity.current_behavior {
                    BehaviorState::Fleeing | BehaviorState::Hunting => rates.energy * 3.0,
                    BehaviorState::Resting => -0.2,
                    _ => rates.energy,
                };
                entity.needs.energy = (entity.needs.energy - energy_drain * delta_seconds).clamp(0.0, 100.0);
            }

            // Reproduction urge increases when basic needs are met
            // Note: rates.reproduction is negative by convention, so we use abs()
            if entity.needs.hunger > 50.0 && entity.needs.thirst > 50.0 && entity.needs.energy > 40.0 {
                entity.needs.reproduction =
                    (entity.needs.reproduction + rates.reproduction.abs() * delta_seconds).min(100.0);
            }

            // Check starvation/dehydration
            if entity.needs.hunger <= 0.0 {
                entity.current_health -= 1.0 * delta_seconds;
            }
            if entity.needs.thirst <= 0.0 {
                entity.current_health -= 2.0 * delta_seconds;
            }
        }

        // Check if dead from starvation
        let should_die = {
            let entity = self.wildlife.get(entity_id).unwrap();
            entity.current_health <= 0.0
        };

        if should_die {
            self.kill_entity(entity_id.to_string(), None, "starvation".to_string());
            return;
        }

        let _t_pre_perc = _t_entity_start.elapsed().as_micros();

        // Refresh perception caches if stale and the entity might actually
        // care.  Each terrain.nearest_* call scans thousands of cells; doing
        // it every tick for every entity was the dominant remaining cost.
        // Thirsty/tired animals need fresh data; sated ones don't.
        {
            let (pos, needs_water_check, needs_shelter_check) = {
                let entity = self.wildlife.get(entity_id).unwrap();
                let water_stale   = now_ms - entity.cached_water_at_ms   > 3_000;
                let shelter_stale = now_ms - entity.cached_shelter_at_ms > 3_000;
                let need_water   = entity.needs.thirst < 70.0;
                let need_shelter = matches!(
                    entity.current_behavior,
                    BehaviorState::Fleeing | BehaviorState::Resting,
                ) || entity.needs.energy < 40.0;
                (entity.position,
                 water_stale   && need_water,
                 shelter_stale && need_shelter)
            };
            if needs_water_check {
                let water = self.terrain.as_ref()
                    .and_then(|t| t.nearest_water(pos.x, pos.z, species.smell_range));
                if let Some(e) = self.wildlife.get_mut(entity_id) {
                    e.cached_water = water;
                    e.cached_water_at_ms = now_ms;
                }
            }
            if needs_shelter_check {
                let shelter = self.terrain.as_ref()
                    .and_then(|t| t.nearest_shelter(pos.x, pos.z, species.sight_range));
                if let Some(e) = self.wildlife.get_mut(entity_id) {
                    e.cached_shelter = shelter;
                    e.cached_shelter_at_ms = now_ms;
                }
            }
        }

        let _t_perc_done = _t_entity_start.elapsed().as_micros();

        // Build environment context (needs immutable borrow of self)
        let context = {
            let entity = self.wildlife.get(entity_id).unwrap();
            self.build_context(entity, species)
        };
        let _t_ctx_done = _t_entity_start.elapsed().as_micros();

        // Select behavior
        let decision = {
            let entity = self.wildlife.get(entity_id).unwrap();
            select_behavior(entity, species, &context)
        };

        // ── Pathfinding: compute next waypoint before mutably borrowing entity ──
        let base_speed = match decision.behavior {
            BehaviorState::Hibernating => 0.0,
            BehaviorState::Fleeing => species.base_speed * species.flee_speed_multiplier,
            BehaviorState::Hunting | BehaviorState::SeekingMate => species.base_speed,
            BehaviorState::Stalking => species.base_speed * 0.5,
            BehaviorState::Foraging | BehaviorState::Wandering => species.base_speed * 0.7,
            _ => 0.0,
        };

        let next_waypoint: Option<Vector3> = if base_speed > 0.0 {
            if let Some(ref terrain) = self.terrain {
                let entity = self.wildlife.get(entity_id).unwrap();
                let entity_pos = entity.position;
                let entity_heading = entity.heading;

                // Determine movement target
                let move_target = match (&decision.target_position, decision.behavior) {
                    (Some(threat), BehaviorState::Fleeing) => {
                        // Flee distance scales with species sight range — a deer that
                        // detects a threat at 120 m should run far, not just 30 m.
                        let _threat_dist = entity_pos.distance_2d(threat);
                        let flee_dist = (species.sight_range * 1.5)
                            .max(80.0)   // At minimum bolt 80 m
                            .min(300.0); // Cap to avoid pathing across the whole map

                        // Prey/hybrid prefers fleeing toward nearby cover/shelter
                        let mut flee_point = None;

                        if species.diet_type == DietType::Prey
                            || species.diet_type == DietType::Hybrid
                        {
                            // Search for shelter within the flee distance
                            let shelter_range = flee_dist.min(120.0);
                            if let Some(shelter) = terrain.nearest_shelter(entity_pos.x, entity_pos.z, shelter_range) {
                                // Only use shelter if it's roughly in the flee direction
                                let to_shelter_x = shelter.x - entity_pos.x;
                                let to_shelter_z = shelter.z - entity_pos.z;
                                let away_x = entity_pos.x - threat.x;
                                let away_z = entity_pos.z - threat.z;
                                let dot = to_shelter_x * away_x + to_shelter_z * away_z;
                                if dot > 0.0 {
                                    flee_point = Some(shelter);
                                }
                            }
                        }

                        // Default: flee directly away from threat
                        if flee_point.is_none() {
                            let dx = entity_pos.x - threat.x;
                            let dz = entity_pos.z - threat.z;
                            let dist = (dx * dx + dz * dz).sqrt().max(0.01);
                            let (bmin, bmax) = terrain.world_bounds();
                            let fx = (entity_pos.x + (dx / dist) * flee_dist).clamp(bmin.x + 1.0, bmax.x - 1.0);
                            let fz = (entity_pos.z + (dz / dist) * flee_dist).clamp(bmin.z + 1.0, bmax.z - 1.0);
                            let fy = terrain.get_elevation(fx, fz) as f64;
                            flee_point = Some(Vector3::new(fx, fy, fz));
                        }

                        flee_point
                    }
                    (Some(target), _) => Some(*target),
                    (None, _) => {
                        // Wandering: reuse existing wander target if still valid
                        let existing = entity.wander_target;
                        let existing_age_ms = now_ms - entity.wander_target_set_at;
                        let reuse = existing.is_some()
                            && existing_age_ms < 8000
                            && existing.unwrap().distance_2d(&entity_pos) > 3.0;

                        if reuse {
                            existing
                        } else {
                            // Generate a new random walkable point
                            let wander_dist = 15.0 + rng.gen_range(0.0..15.0);
                            let mut target = None;
                            for _ in 0..5 {
                                let angle: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
                                let wx = entity_pos.x + angle.sin() * wander_dist;
                                let wz = entity_pos.z + angle.cos() * wander_dist;
                                if terrain.is_walkable(wx, wz) {
                                    let wy = terrain.get_elevation(wx, wz) as f64;
                                    target = Some(Vector3::new(wx, wy, wz));
                                    break;
                                }
                            }
                            // If nothing walkable found, wander based on heading
                            if target.is_none() {
                                let rad = entity_heading.to_radians();
                                let wx = entity_pos.x + rad.sin() * wander_dist;
                                let wz = entity_pos.z + rad.cos() * wander_dist;
                                if terrain.is_walkable(wx, wz) {
                                    let wy = terrain.get_elevation(wx, wz) as f64;
                                    target = Some(Vector3::new(wx, wy, wz));
                                }
                            }
                            // Persist the new wander target on the entity
                            if let Some(t) = target {
                                if let Some(e) = self.wildlife.get_mut(entity_id) {
                                    e.wander_target = Some(t);
                                    e.wander_target_set_at = now_ms;
                                }
                            }
                            target
                        }
                    }
                };

                if let Some(target) = move_target {
                    // Check if we need a new path — tolerances scale by behavior
                    let is_urgent = matches!(
                        decision.behavior,
                        BehaviorState::Hunting | BehaviorState::Fleeing
                            | BehaviorState::Stalking | BehaviorState::SeekingMate
                    );
                    let target_move_tolerance = if is_urgent { 5.0 } else { 20.0 };
                    let stale_ms = if is_urgent { 2000 } else { 10000 };

                    let needs_new_path = match self.entity_paths.get(entity_id) {
                        None => true,
                        Some(p) => {
                            p.current_index >= p.waypoints.len()
                                || target.distance_2d(&p.target) > target_move_tolerance
                                || now_ms - p.computed_at_ms > stale_ms
                        }
                    };

                    if needs_new_path {
                        // Defer A* — collected and run in parallel at end of tick.
                        // Entity uses existing (possibly stale) path this tick or
                        // falls back to heading-based movement if no path exists.
                        self.pending_path_requests.push(PathRequest {
                            entity_id:       entity_id.to_string(),
                            from:            entity_pos,
                            to:              target,
                            requested_at_ms: now_ms,
                        });
                    }

                    self.entity_paths
                        .get(entity_id)
                        .and_then(|p| p.waypoints.get(p.current_index))
                        .copied()
                } else {
                    None
                }
            } else {
                None // No terrain — will fall back to heading-based movement
            }
        } else {
            None
        };

        // ── Apply behavior and movement ──
        {
            let entity = self.wildlife.get_mut(entity_id).unwrap();
            // Clear wander target when switching away from wandering
            if decision.behavior != BehaviorState::Wandering && entity.wander_target.is_some() {
                entity.wander_target = None;
            }
            entity.current_behavior = decision.behavior;
            entity.target_entity_id = decision.target_id;

            let from_position = entity.position;
            let mut speed = base_speed;

            // Apply heat stress speed penalty
            let temperature = self.current_climate.as_ref().map(|c| c.temperature).unwrap_or(0.0);
            if temperature > 0.6 && entity.current_behavior != BehaviorState::Hibernating {
                speed *= 0.7;
            }

            if speed > 0.0 {
                if let Some(waypoint) = next_waypoint {
                    // ── Terrain-aware: move toward path waypoint ──
                    let dx = waypoint.x - entity.position.x;
                    let dz = waypoint.z - entity.position.z;
                    let dist = (dx * dx + dz * dz).sqrt();

                    // Apply terrain movement cost (road = faster, rubble = slower)
                    if let Some(ref terrain) = self.terrain {
                        let cost = terrain.get_movement_cost(entity.position.x, entity.position.z);
                        if cost.is_finite() && cost > 0.0 {
                            speed /= cost as f64;
                        }
                    }

                    if dist > 0.01 {
                        let move_dist = speed * delta_seconds;
                        if move_dist >= dist {
                            entity.position.x = waypoint.x;
                            entity.position.z = waypoint.z;
                        } else {
                            entity.position.x += (dx / dist) * move_dist;
                            entity.position.z += (dz / dist) * move_dist;
                        }
                        entity.heading = dx.atan2(dz).to_degrees();
                    }

                    // Set Y from terrain elevation
                    if let Some(ref terrain) = self.terrain {
                        entity.position.y = terrain.get_elevation(entity.position.x, entity.position.z) as f64;
                    }
                } else {
                    // ── Fallback: heading-based movement (no terrain or no path) ──
                    let heading = match (&decision.target_position, entity.current_behavior) {
                        (Some(target), BehaviorState::Fleeing) => {
                            flee_direction(&entity.position, target)
                        }
                        (Some(target), _) => {
                            approach_direction(&entity.position, target)
                        }
                        (None, _) => {
                            wander_direction(
                                &entity.position,
                                entity.home_position.as_ref(),
                                entity.heading,
                                rng,
                            )
                        }
                    };

                    let rad = heading.to_radians();
                    let dx = rad.sin() * speed * delta_seconds;
                    let dz = rad.cos() * speed * delta_seconds;

                    // Check walkability before moving
                    let new_x = entity.position.x + dx;
                    let new_z = entity.position.z + dz;

                    if let Some(ref terrain) = self.terrain {
                        if terrain.is_walkable(new_x, new_z) {
                            entity.position.x = new_x;
                            entity.position.z = new_z;
                        } else if terrain.is_walkable(new_x, entity.position.z) {
                            entity.position.x = new_x;
                        } else if terrain.is_walkable(entity.position.x, new_z) {
                            entity.position.z = new_z;
                        }
                        entity.position.y = terrain.get_elevation(entity.position.x, entity.position.z) as f64;
                    } else {
                        entity.position.x = new_x;
                        entity.position.z = new_z;
                    }

                    entity.heading = heading;
                }

                // Clamp to bounds
                if let Some(ref terrain) = self.terrain {
                    let (bmin, bmax) = terrain.world_bounds();
                    entity.position.x = entity.position.x.clamp(bmin.x, bmax.x);
                    entity.position.z = entity.position.z.clamp(bmin.z, bmax.z);
                } else {
                    entity.position.x = entity.position.x.clamp(self.bounds_min.x, self.bounds_max.x);
                    entity.position.z = entity.position.z.clamp(self.bounds_min.z, self.bounds_max.z);
                }
            }

            // Emit Move only when the entity actually moved — flooding 2400
            // identical "still-here" updates per tick at 10 Hz pushed Redis's
            // pub/sub output buffer past its limit and dropped the subscriber.
            // Behaviour change (e.g. idle → fleeing) is also worth a tick.
            let position_changed =
                (entity.position.x - from_position.x).abs() > 0.001 ||
                (entity.position.z - from_position.z).abs() > 0.001;
            if position_changed {
                self.pending_events.push(WildlifeEvent::Move {
                    entity_id: entity.id.clone(),
                    from_position,
                    position: entity.position,
                    heading: entity.heading,
                    behavior: entity.current_behavior,
                });
            }
        }

        // Advance path index if we reached the waypoint
        if next_waypoint.is_some() {
            if let Some(entity) = self.wildlife.get(entity_id) {
                if let Some(wp) = next_waypoint {
                    if entity.position.distance_2d(&wp) < 1.5 {
                        if let Some(path) = self.entity_paths.get_mut(entity_id) {
                            path.current_index += 1;
                            // Clear wander target if we reached the end of a wander path
                            if path.current_index >= path.waypoints.len() {
                                if let Some(e) = self.wildlife.get_mut(entity_id) {
                                    e.wander_target = None;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Handle behavior-specific actions and aging
        let (should_mate, plant_to_eat, stage_changed) = {
            let entity = self.wildlife.get_mut(entity_id).unwrap();

            let mut plant_to_eat: Option<String> = None;
            let mut stage_changed = false;

            match entity.current_behavior {
                BehaviorState::Drinking => {
                    entity.needs.thirst = (entity.needs.thirst + 10.0 * delta_seconds).min(100.0);
                }
                BehaviorState::Eating => {
                    // Eating is handled separately - we need to consume the target
                    if let Some(target_id) = &entity.target_entity_id {
                        plant_to_eat = Some(target_id.clone());
                    }
                }
                BehaviorState::Foraging => {
                    // If close enough to food target, eat it
                    if let Some(target_id) = &entity.target_entity_id {
                        plant_to_eat = Some(target_id.clone());
                    }
                }
                BehaviorState::Resting => {
                    entity.needs.energy = (entity.needs.energy + 5.0 * delta_seconds).min(100.0);
                }
                _ => {}
            }

            // Update age
            entity.age += delta_seconds;
            if !entity.is_mature && entity.age >= species.maturity_time {
                entity.is_mature = true;
            }
            let next_stage = Self::age_stage_for(entity.age, species.maturity_time);
            if entity.age_stage != next_stage {
                entity.age_stage = next_stage;
                stage_changed = true;
            }

            entity.last_update_at = now_ms;

            // Check if mating should initiate pregnancy
            let should_mate = entity.current_behavior == BehaviorState::Mating
                && entity.sex == Sex::Female
                && !entity.is_pregnant
                && entity.target_entity_id.is_some();

            (should_mate, plant_to_eat, stage_changed)
        };

        if stage_changed {
            if let Some(entity) = self.wildlife.get_mut(entity_id) {
                Self::apply_stat_scaling(entity, species);
            }
        }

        // Handle plant eating (separate block to avoid borrow issues)
        if let Some(plant_id) = plant_to_eat {
            self.eat_plant(entity_id, &plant_id);
        }

        // Handle mating (separate block to avoid borrow issues)
        if should_mate {
            self.process_mating(entity_id, now_ms, species.gestation_time);
        }

        // Check pregnancy (using the flag we computed earlier)
        if should_check_birth {
            self.give_birth(entity_id.to_string(), now_ms, rng);
        }

        let _t_total = _t_entity_start.elapsed().as_micros();
        if _t_total > 5_000 {
            tracing::warn!(
                "ENTITY_SLOW total={} pre_perc={} after_perc={} after_ctx={} species={}",
                _t_total, _t_pre_perc, _t_perc_done, _t_ctx_done, species.id,
            );
        }
    }

    fn build_context(&self, entity: &WildlifeEntity, species: &WildlifeSpecies) -> EnvironmentContext {
        let max_range = species.sight_range.max(species.hearing_range).max(species.smell_range);
        let max_range_sq = max_range * max_range;

        let mut threats = Vec::new();
        let mut prey = Vec::new();
        let mut mates = Vec::new();

        // Check other wildlife — use the spatial grid so we only iterate
        // entities in cells that overlap our perception circle, not all 1500.
        let mut neighbour_ids: Vec<String> = Vec::new();
        self.spatial_query(entity.position, max_range, |id| {
            if id != entity.id { neighbour_ids.push(id.to_string()); }
        });

        for other_id in &neighbour_ids {
            let other = match self.wildlife.get(other_id) {
                Some(e) if e.is_alive => e,
                _ => continue,
            };

            let dx = entity.position.x - other.position.x;
            let dy = entity.position.y - other.position.y;
            let dz = entity.position.z - other.position.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            if dist_sq > max_range_sq { continue; }
            let distance = dist_sq.sqrt();

            let other_species = match get_species(&other.species_id) {
                Some(s) => s,
                None => continue,
            };

            let perceived = PerceivedEntity {
                id: other.id.clone(),
                position: other.position,
                distance,
                size_class: other_species.size_class,
                diet_type: Some(other_species.diet_type),
                species_id: Some(other.species_id.clone()),
                is_player: false,
            };

            if is_threat(species, other_species.size_class, Some(other_species.diet_type), false) {
                threats.push(perceived.clone());
            }
            if is_prey(species, other_species.size_class) {
                prey.push(perceived.clone());
            }
            // Only opposite sex can be mates, and females can't be pregnant already
            if other.species_id == entity.species_id
                && other.is_mature
                && other.sex != entity.sex
                && (entity.sex == Sex::Male || !other.is_pregnant)
            {
                mates.push(perceived);
            }
        }

        // Check players (always potential threats)
        for player in &self.player_positions {
            let distance = entity.position.distance_to(&player.position);
            if distance > max_range {
                continue;
            }

            if is_threat(species, SizeClass::Medium, None, true) {
                threats.push(PerceivedEntity {
                    id: player.id.clone(),
                    position: player.position,
                    distance,
                    size_class: SizeClass::Medium,
                    diet_type: None,
                    species_id: None,
                    is_player: true,
                });
            }
        }

        // Sort by distance
        threats.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        prey.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        mates.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

        // Water sources — read from the entity's cached value (refreshed in
        // update_wildlife_entity at most every few seconds) instead of
        // scanning the terrain grid every tick.
        let nearby_water: Vec<WaterSource> = if let Some(water_pos) = entity.cached_water {
            let distance = entity.position.distance_2d(&water_pos);
            if distance <= species.smell_range {
                vec![WaterSource { position: water_pos, distance }]
            } else {
                Vec::new()
            }
        } else if self.terrain.is_none() {
            // No terrain — fall back to hardcoded water sources (cheap)
            self.water_sources
                .iter()
                .map(|pos| WaterSource {
                    position: *pos,
                    distance: entity.position.distance_to(pos),
                })
                .filter(|w| w.distance <= species.smell_range)
                .collect()
        } else {
            Vec::new()
        };

        // Food (plants for herbivores) — use the plant spatial grid so we
        // only iterate cells overlapping the perception circle, not all 105k
        // plants.  Was the dominant per-entity cost (25 ms/herbivore) before.
        let nearby_food: Vec<PerceivedEntity> = if species.is_herbivore {
            let smell_sq = species.smell_range * species.smell_range;
            let mut food = Vec::new();
            self.plant_spatial_query(entity.position, species.smell_range, |id| {
                if let Some(p) = self.plants.get(id) {
                    if !p.is_alive { return; }
                    if !matches!(
                        p.current_stage,
                        PlantGrowthStage::Mature | PlantGrowthStage::Growing | PlantGrowthStage::Flowering,
                    ) { return; }
                    let dx = entity.position.x - p.position.x;
                    let dy = entity.position.y - p.position.y;
                    let dz = entity.position.z - p.position.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    if dist_sq > smell_sq { return; }
                    food.push(PerceivedEntity {
                        id:         p.id.clone(),
                        position:   p.position,
                        distance:   dist_sq.sqrt(),
                        size_class: SizeClass::Tiny,
                        diet_type:  None,
                        species_id: Some(p.species_id.clone()),
                        is_player:  false,
                    });
                }
            });
            food
        } else {
            Vec::new()
        };

        // Use per-cell terrain biome if available, otherwise zone-level biome
        let local_biome = self
            .terrain
            .as_ref()
            .map(|t| terrain_biome_to_zone_biome(t.get_biome_at(entity.position.x, entity.position.z)))
            .unwrap_or(self.biome);

        let biome_comfort = species
            .biome_preferences
            .iter()
            .find(|p| p.biome == local_biome)
            .map(|p| p.comfort)
            .unwrap_or(30.0);

        let climate = self.current_climate.as_ref();
        let weather = self.current_weather.as_ref();

        // Weather hazards from active events
        let mut hazards = Vec::new();
        if let Some(w) = weather {
            for event in &w.active_events {
                let pos = Vector3 {
                    x: event.position[0],
                    y: event.position[1],
                    z: event.position[2],
                };
                let distance = entity.position.distance_to(&pos);

                let radius = match event.event_type {
                    WeatherEventType::Storm { radius, .. } => Some(radius),
                    WeatherEventType::Tornado { radius, .. } => Some(radius),
                    _ => None,
                };

                if let Some(r) = radius {
                    // Consider hazard if within radius + a buffer
                    if distance <= r + 25.0 {
                        hazards.push(PerceivedEntity {
                            id: event.id.clone(),
                            position: pos,
                            distance,
                            size_class: SizeClass::Large,
                            diet_type: None,
                            species_id: None,
                            is_player: false,
                        });
                    }
                }
            }
            hazards.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        }

        // Terrain-aware context: shelter, road, cover
        let (nearby_shelter, on_road, near_cover) = if let Some(ref terrain) = self.terrain {
            let shelter = terrain.nearest_shelter(
                entity.position.x,
                entity.position.z,
                species.sight_range,
            );
            let road = terrain.is_road(entity.position.x, entity.position.z);
            let cover = terrain.get_biome_at(entity.position.x, entity.position.z) == crate::terrain::TerrainBiome::Forest
                || terrain.is_shelter(entity.position.x, entity.position.z);
            (shelter, road, cover)
        } else {
            (None, false, false)
        };

        EnvironmentContext {
            current_biome: local_biome,
            biome_comfort,
            time_of_day: climate.map(|c| c.time_of_day).unwrap_or(12.0),
            is_night: climate.map(|c| c.is_night).unwrap_or(false),
            nearby_threats: threats,
            nearby_prey: prey,
            nearby_food,
            nearby_water,
            nearby_mates: mates,
            season: climate.map(|c| c.season).unwrap_or(Season::Spring),
            temperature: climate.map(|c| c.temperature).unwrap_or(0.0),
            nearby_hazards: hazards,
            nearby_shelter,
            on_road,
            near_cover,
        }
    }

    fn process_hunting(&mut self, now_ms: i64) {
        let mut attacks: Vec<(String, String, f64)> = Vec::new();

        for entity in self.wildlife.values() {
            if !entity.is_alive {
                continue;
            }

            if entity.current_behavior != BehaviorState::Hunting {
                continue;
            }

            if now_ms < entity.attack_cooldown_until {
                continue;
            }

            let species = match get_species(&entity.species_id) {
                Some(s) => s,
                None => continue,
            };

            let target_id = match &entity.target_entity_id {
                Some(id) => id.clone(),
                None => continue,
            };

            let target = match self.wildlife.get(&target_id) {
                Some(t) if t.is_alive => t,
                _ => continue,
            };

            let distance = entity.position.distance_to(&target.position);
            if distance <= species.attack_range {
                attacks.push((
                    entity.id.clone(),
                    target_id,
                    entity.attack_damage,
                ));
            }
        }

        // Apply attacks
        for (attacker_id, target_id, damage) in attacks {
            let attacker = self.wildlife.get_mut(&attacker_id).unwrap();
            let species = get_species(&attacker.species_id).unwrap();
            attacker.attack_cooldown_until =
                now_ms + (species.attack_cooldown * 1000.0) as i64;

            let attacker_pos = attacker.position;

            // Apply damage to target (in separate block to release borrow)
            let target_died = {
                let target = self.wildlife.get_mut(&target_id).unwrap();
                target.current_health -= damage;
                target.current_health <= 0.0
            };

            // Award XP for the hit
            self.award_experience(&attacker_id, 2.0);

            self.pending_events.push(WildlifeEvent::Attack {
                attacker_id: attacker_id.clone(),
                target_id: target_id.clone(),
                damage,
                position: attacker_pos,
            });

            if target_died {
                // Calculate food value from prey before killing it
                let prey_species = get_species(&self.wildlife.get(&target_id).unwrap().species_id);
                let food_value = prey_species
                    .as_ref()
                    .map(|s| {
                        // Food value based on prey size class
                        match s.size_class {
                            SizeClass::Tiny => 25.0,
                            SizeClass::Small => 40.0,
                            SizeClass::Medium => 60.0,
                            SizeClass::Large => 80.0,
                            SizeClass::Huge => 100.0,
                        }
                    })
                    .unwrap_or(30.0);
                let size_xp = prey_species.map(|s| Self::size_class_xp(s.size_class)).unwrap_or(4.0);

                self.kill_entity(target_id.clone(), Some(attacker_id.clone()), "predation".to_string());
                self.award_experience(&attacker_id, 8.0 + size_xp);

                // Predator eats the kill - gain hunger satisfaction
                if let Some(attacker) = self.wildlife.get_mut(&attacker_id) {
                    let old_hunger = attacker.needs.hunger;
                    attacker.needs.hunger = (attacker.needs.hunger + food_value).min(100.0);
                    attacker.target_entity_id = None; // Clear target after eating
                    tracing::info!(
                        "Predator {} ate prey, hunger {:.0} -> {:.0} (+{:.0})",
                        attacker_id,
                        old_hunger,
                        attacker.needs.hunger,
                        food_value
                    );
                }
            } else {
                // Target starts fleeing
                if let Some(target) = self.wildlife.get_mut(&target_id) {
                    target.current_behavior = BehaviorState::Fleeing;
                    target.fleeing_until = now_ms + 10_000;
                }
                self.award_experience(&target_id, 1.0);
            }
        }
    }

    /// Handle herbivore eating a plant
    fn eat_plant(&mut self, entity_id: &str, plant_id: &str) {
        // Get plant info first
        let (food_value, plant_position) = {
            let plant = match self.plants.get(plant_id) {
                Some(p) if p.is_alive => p,
                _ => return,
            };

            let species = match get_plant_species(&plant.species_id) {
                Some(s) => s,
                None => return,
            };

            // Get food value for current stage
            let stage_config = species.growth_stages.get(plant.stage_index);
            let food_value = match stage_config {
                Some(config) if config.can_be_eaten => config.food_value,
                _ => return, // Can't eat this plant at this stage
            };

            (food_value, plant.position)
        };

        // Check if entity is close enough
        let entity_pos = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) if e.is_alive => e,
                _ => return,
            };
            entity.position
        };

        let distance = entity_pos.distance_to(&plant_position);
        if distance > 3.0 {
            return; // Too far to eat
        }

        // Apply food value to entity
        if let Some(entity) = self.wildlife.get_mut(entity_id) {
            entity.needs.hunger = (entity.needs.hunger + food_value).min(100.0);
            entity.target_entity_id = None; // Clear target after eating
        }

        // Damage/consume the plant
        let plant = self.plants.get_mut(plant_id).unwrap();
        plant.is_alive = false;
        plant.current_stage = PlantGrowthStage::Dead;
        self.plant_grid_dirty = true;

        // Emit event
        self.pending_events.push(WildlifeEvent::PlantEaten {
            plant_id: plant_id.to_string(),
            wildlife_id: entity_id.to_string(),
            food_value,
        });

        tracing::debug!(
            "Wildlife {} ate plant {} for {} hunger",
            entity_id,
            plant_id,
            food_value
        );
    }

    fn process_mating(&mut self, female_id: &str, now_ms: i64, gestation_time: f64) {
        // First, gather what we need (immutable borrow)
        let (mate_id, female_pos, female_name, species_id) = {
            let entity = match self.wildlife.get(female_id) {
                Some(e) => e,
                None => return,
            };
            let mate_id = match &entity.target_entity_id {
                Some(id) => id.clone(),
                None => return,
            };
            (mate_id, entity.position, entity.name.clone(), entity.species_id.clone())
        };

        // Check if mate is nearby (separate borrow)
        let mate_nearby = self.wildlife.get(&mate_id)
            .map(|mate| {
                mate.is_alive
                    && mate.sex == Sex::Male
                    && female_pos.distance_to(&mate.position) < 3.0
            })
            .unwrap_or(false);

        if !mate_nearby {
            return;
        }

        // Initiate pregnancy (mutable borrow)
        if let Some(entity) = self.wildlife.get_mut(female_id) {
            entity.is_pregnant = true;
            entity.pregnancy_ends_at = Some(now_ms + (gestation_time * 1000.0) as i64);
            entity.needs.reproduction = 0.0;
        }

        // Also reset the male's reproduction need
        if let Some(mate) = self.wildlife.get_mut(&mate_id) {
            mate.needs.reproduction = 0.0;
        }

        tracing::info!(
            "{} ({}) is now pregnant, due in {:.0}s",
            female_name,
            species_id,
            gestation_time
        );
    }

    fn kill_entity(&mut self, entity_id: String, killer_id: Option<String>, cause: String) {
        // Get killer species if applicable
        let killer_species = killer_id.as_ref().and_then(|kid| {
            self.wildlife.get(kid).map(|k| k.species_id.clone())
        });

        let entity = match self.wildlife.get_mut(&entity_id) {
            Some(e) => e,
            None => return,
        };

        let health_at_death = entity.current_health;
        let age = entity.age;
        let name = entity.name.clone();

        entity.is_alive = false;
        entity.current_behavior = BehaviorState::Dead;

        self.pending_events.push(WildlifeEvent::Death {
            entity_id: entity.id.clone(),
            species_id: entity.species_id.clone(),
            name,
            position: entity.position,
            zone_id: self.zone_id.clone(),
            killer_id,
            killer_species,
            cause,
            age,
            health_at_death,
        });
    }

    fn give_birth(&mut self, parent_id: String, now_ms: i64, rng: &mut impl Rng) {
        let parent = match self.wildlife.get_mut(&parent_id) {
            Some(p) => p,
            None => return,
        };

        let species = match get_species(&parent.species_id) {
            Some(s) => s,
            None => return,
        };

        let offspring_count =
            rng.gen_range(species.offspring_min..=species.offspring_max);

        let parent_pos = parent.position;
        let species_id = parent.species_id.clone();

        parent.is_pregnant = false;
        parent.pregnancy_ends_at = None;
        parent.needs.reproduction = 0.0;

        let mut offspring_ids = Vec::new();

        for _ in 0..offspring_count {
            let offset_x = rng.gen_range(-4.0..4.0);
            let offset_z = rng.gen_range(-4.0..4.0);

            let child_x = parent_pos.x + offset_x;
            let child_z = parent_pos.z + offset_z;
            let child_y = self
                .terrain
                .as_ref()
                .map(|t| t.get_elevation(child_x, child_z) as f64)
                .unwrap_or(parent_pos.y);

            let child = self.spawn_entity(
                &species_id,
                Vector3::new(child_x, child_y, child_z),
                now_ms,
                false,
            );

            if let Some(mut child) = child {
                child.is_mature = false;
                child.age = 0.0;
                offspring_ids.push(child.id.clone());
                self.wildlife.insert(child.id.clone(), child);
            }
        }

        self.pending_events.push(WildlifeEvent::Birth {
            parent_id,
            offspring_ids,
            position: parent_pos,
            zone_id: self.zone_id.clone(),
        });
    }

    fn check_spawns(&mut self, now_ms: i64, _rng: &mut impl Rng) {
        // Emergency respawns when population drops critically low.
        // Thresholds scale with zone area so a 6 km² map doesn't wait
        // until 3 deer remain before replenishing.
        let area = self.zone_area_km2();

        // (species, min_density/km², respawn_males_density, respawn_females_density, min_pop, min_m, min_f)
        let specs: &[(&str, f64, f64, f64, usize, usize, usize)] = &[
            ("rabbit", 1.5,  1.0, 2.0,  3, 2, 3),
            ("fox",    0.5,  0.5, 0.5,  3, 1, 2),
            ("deer",   0.8,  0.8, 1.2,  3, 1, 2),
            ("wolf",   0.3,  0.3, 0.5,  2, 1, 2),
            ("boar",   0.5,  0.5, 0.8,  3, 1, 2),
        ];

        for &(species_id, min_den, rm_den, rf_den, min_pop, min_m, min_f) in specs {
            let species = match get_species(species_id) {
                Some(s) => s,
                None => continue,
            };

            // When terrain is available, check if ANY cell matches a preferred biome.
            // Otherwise fall back to zone-level biome check.
            let biome_ok = if self.terrain.is_some() {
                !species.biome_preferences.is_empty()
            } else {
                species.biome_preferences.iter().any(|p| p.biome == self.biome)
            };

            if !biome_ok {
                continue;
            }

            let min_population = ((min_den * area).round() as usize).max(min_pop);
            let respawn_males  = ((rm_den * area).round() as usize).max(min_m);
            let respawn_females = ((rf_den * area).round() as usize).max(min_f);

            let alive_count = self
                .wildlife
                .values()
                .filter(|e| e.species_id == species_id && e.is_alive)
                .count();

            if alive_count < min_population {
                tracing::info!(
                    "Population of {} critically low ({} < {}), spawning reinforcements ({}M + {}F)",
                    species_id,
                    alive_count,
                    min_population,
                    respawn_males,
                    respawn_females
                );
                self.spawn_population(species_id, respawn_males, respawn_females, now_ms);
            }
        }
    }

    fn find_spawn_position(&self, config: &SpawnConfig, rng: &mut impl Rng) -> Option<Vector3> {
        for _ in 0..10 {
            let pos = Vector3::new(
                rng.gen_range(self.bounds_min.x..self.bounds_max.x),
                0.0,
                rng.gen_range(self.bounds_min.z..self.bounds_max.z),
            );

            // Check player distance
            let too_close_to_player = self
                .player_positions
                .iter()
                .any(|p| pos.distance_to(&p.position) < config.min_distance_from_players);

            if too_close_to_player {
                continue;
            }

            // Check same-species distance
            let too_close_to_same = self
                .wildlife
                .values()
                .filter(|e| e.species_id == config.species_id && e.is_alive)
                .any(|e| pos.distance_to(&e.position) < config.min_distance_between);

            if too_close_to_same {
                continue;
            }

            return Some(pos);
        }

        None
    }

    fn spawn_entity(
        &mut self,
        species_id: &str,
        position: Vector3,
        now_ms: i64,
        as_adult: bool,
    ) -> Option<WildlifeEntity> {
        // Random sex
        let sex = if rand::thread_rng().gen_bool(0.5) {
            Sex::Female
        } else {
            Sex::Male
        };
        self.spawn_entity_with_sex(species_id, position, now_ms, as_adult, sex)
    }

    fn spawn_entity_with_sex(
        &mut self,
        species_id: &str,
        position: Vector3,
        now_ms: i64,
        as_adult: bool,
        sex: Sex,
    ) -> Option<WildlifeEntity> {
        let species = get_species(species_id)?;

        let id = format!("wildlife_{}_{}", species_id, self.next_entity_id);
        self.next_entity_id += 1;

        let mut entity = WildlifeEntity {
            id,
            species_id: species_id.to_string(),
            name: format!("a {}", species.name.to_lowercase()),

            position,
            heading: rand::thread_rng().gen_range(0.0..360.0),
            zone_id: self.zone_id.clone(),
            current_biome: self
                .terrain
                .as_ref()
                .map(|t| terrain_biome_to_zone_biome(t.get_biome_at(position.x, position.z)))
                .unwrap_or(self.biome),

            is_alive: true,
            current_health: species.max_health,
            max_health: species.max_health,
            attack_damage: species.attack_damage,
            needs: WildlifeNeeds::default(),

            current_behavior: BehaviorState::Idle,
            target_entity_id: None,
            home_position: Some(position),

            last_update_at: now_ms,
            attack_cooldown_until: 0,
            fleeing_until: 0,

            sex,
            is_pregnant: false,
            pregnancy_ends_at: None,
            age: if as_adult { species.maturity_time } else { 0.0 },
            is_mature: as_adult,
            age_stage: if as_adult { WildlifeAgeStage::Adult } else { WildlifeAgeStage::Juvenile },
            level: 1,
            experience: 0.0,
            experience_to_next: Self::experience_to_next(1),

            in_combat: false,
            last_hostile_at: 0,

            wander_target: None,
            wander_target_set_at: 0,

            cached_water: None,
            cached_water_at_ms: 0,
            cached_shelter: None,
            cached_shelter_at_ms: 0,
            last_update_tick_ms: 0,
        };

        Self::apply_stat_scaling(&mut entity, &species);

        // Emit Spawn event so the game server knows this entity exists
        self.pending_events.push(WildlifeEvent::Spawn {
            entity_id: entity.id.clone(),
            species_id: entity.species_id.clone(),
            position: entity.position,
            zone_id: self.zone_id.clone(),
        });

        Some(entity)
    }

    /// Take pending events (clears the internal buffer)
    pub fn take_events(&mut self) -> Vec<WildlifeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Re-emit Spawn events for every living entity and plant.
    ///
    /// Called when the game server restarts (sends ZoneInfo for a zone we
    /// already have) so the bridge can rebuild its entity registry.
    pub fn re_announce_all(&mut self) {
        let wildlife_count = self.wildlife.values().filter(|e| e.is_alive).count();
        info!(
            "Re-announcing {} living wildlife for zone {}",
            wildlife_count, self.zone_id
        );

        for entity in self.wildlife.values().filter(|e| e.is_alive) {
            self.pending_events.push(WildlifeEvent::Spawn {
                entity_id:  entity.id.clone(),
                species_id: entity.species_id.clone(),
                position:   entity.position,
                zone_id:    self.zone_id.clone(),
            });
        }

        // Plants are intentionally NOT re-announced.  They are static; the
        // bridge streams them to each player once on join via
        // streamPlantsOnJoin and the client caches them locally.  Re-emitting
        // 100k+ PlantSpawn events every 15 s on a slow tick was a major
        // contributor to over-budget warnings.
    }

    /// Spawn an initial population for a species
    pub fn spawn_population(
        &mut self,
        species_id: &str,
        males: usize,
        females: usize,
        now_ms: i64,
    ) -> usize {
        let mut rng = rand::thread_rng();
        let mut spawned = 0;

        let species = match get_species(species_id) {
            Some(s) => s,
            None => return 0,
        };

        let sexes: Vec<Sex> = std::iter::repeat(Sex::Male)
            .take(males)
            .chain(std::iter::repeat(Sex::Female).take(females))
            .collect();

        for sex in sexes {
            let pos = self.find_walkable_spawn_position(
                &mut rng,
                Some(&species.biome_preferences),
            );
            let pos = match pos {
                Some(p) => p,
                None => continue,
            };

            if let Some(entity) = self.spawn_entity_with_sex(species_id, pos, now_ms, true, sex) {
                self.wildlife.insert(entity.id.clone(), entity);
                spawned += 1;
            }
        }

        spawned
    }

    /// Find a walkable spawn position, using terrain if available.
    /// If `biome_prefs` is provided, only positions matching one of the preferred biomes
    /// (by terrain cell) are accepted.
    fn find_walkable_spawn_position(
        &self,
        rng: &mut impl Rng,
        biome_prefs: Option<&[BiomePreference]>,
    ) -> Option<Vector3> {
        for _ in 0..30 {
            let x = rng.gen_range(self.bounds_min.x..self.bounds_max.x);
            let z = rng.gen_range(self.bounds_min.z..self.bounds_max.z);

            if let Some(ref terrain) = self.terrain {
                if !terrain.is_walkable(x, z) {
                    continue;
                }

                // Check biome compatibility at this cell
                if let Some(prefs) = biome_prefs {
                    let cell_biome = terrain_biome_to_zone_biome(terrain.get_biome_at(x, z));
                    let biome_match = prefs.iter().any(|p| p.biome == cell_biome);
                    if !biome_match {
                        continue;
                    }
                }

                let y = terrain.get_elevation(x, z) as f64;
                return Some(Vector3::new(x, y, z));
            } else {
                return Some(Vector3::new(x, 0.0, z));
            }
        }
        None
    }

    /// Update player positions from game server
    pub fn update_players(&mut self, players: Vec<PlayerPosition>) {
        self.player_positions = players;
    }

    /// Handle damage from player attack
    pub fn player_attacked(&mut self, target_id: &str, damage: f64, attacker_id: &str) {
        if let Some(entity) = self.wildlife.get_mut(target_id) {
            entity.current_health -= damage;
            entity.current_behavior = BehaviorState::Fleeing;
            entity.target_entity_id = Some(attacker_id.to_string());
            entity.fleeing_until = chrono::Utc::now().timestamp_millis() + 10_000;

            if entity.current_health <= 0.0 {
                self.kill_entity(
                    target_id.to_string(),
                    Some(attacker_id.to_string()),
                    "player".to_string(),
                );
            } else {
                self.award_experience(target_id, 3.0);
            }
        }
    }

    fn age_stage_for(age: f64, maturity_time: f64) -> WildlifeAgeStage {
        if age < maturity_time {
            WildlifeAgeStage::Juvenile
        } else if age < maturity_time * ELDER_AGE_MULTIPLIER {
            WildlifeAgeStage::Adult
        } else {
            WildlifeAgeStage::Elder
        }
    }

    fn experience_to_next(level: u32) -> f64 {
        50.0 + (level as f64 * 25.0)
    }

    fn apply_stat_scaling(entity: &mut WildlifeEntity, species: &WildlifeSpecies) {
        let age_stage = Self::age_stage_for(entity.age, species.maturity_time);
        let (age_health_mult, age_damage_mult) = match age_stage {
            WildlifeAgeStage::Juvenile => (0.75, 0.7),
            WildlifeAgeStage::Adult => (1.0, 1.0),
            WildlifeAgeStage::Elder => (0.85, 0.9),
        };

        let level_index = entity.level.saturating_sub(1) as f64;
        let level_health_mult = 1.0 + LEVEL_HEALTH_BONUS * level_index;
        let level_damage_mult = 1.0 + LEVEL_DAMAGE_BONUS * level_index;

        let new_max = species.max_health * age_health_mult * level_health_mult;
        let new_damage = species.attack_damage * age_damage_mult * level_damage_mult;

        let ratio = if entity.max_health > 0.0 {
            (entity.current_health / entity.max_health).clamp(0.0, 1.0)
        } else {
            1.0
        };

        entity.max_health = new_max.max(1.0);
        entity.current_health = (entity.max_health * ratio).max(1.0);
        entity.attack_damage = new_damage.max(0.1);
        entity.age_stage = age_stage;
    }

    fn award_experience(&mut self, entity_id: &str, amount: f64) {
        let (species_id, level, exp, exp_next) = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            (entity.species_id.clone(), entity.level, entity.experience, entity.experience_to_next)
        };

        let species = match get_species(&species_id) {
            Some(s) => s,
            None => return,
        };

        let entity = match self.wildlife.get_mut(entity_id) {
            Some(e) => e,
            None => return,
        };

        let mut new_exp = exp + amount;
        let mut new_level = level;
        let mut next = exp_next;

        while new_exp >= next {
            new_exp -= next;
            new_level += 1;
            next = Self::experience_to_next(new_level);
        }

        entity.level = new_level;
        entity.experience = new_exp;
        entity.experience_to_next = next;

        Self::apply_stat_scaling(entity, &species);
    }

    fn size_class_xp(size_class: SizeClass) -> f64 {
        match size_class {
            SizeClass::Tiny => 2.0,
            SizeClass::Small => 4.0,
            SizeClass::Medium => 6.0,
            SizeClass::Large => 8.0,
            SizeClass::Huge => 10.0,
        }
    }

    // ========================================================================
    // Plant Methods
    // ========================================================================

    fn update_plant(&mut self, plant_id: &str, delta_seconds: f64) {
        let plant = match self.plants.get(plant_id) {
            Some(p) => p,
            None => return,
        };

        if !plant.is_alive {
            return;
        }

        let species = match get_plant_species(&plant.species_id) {
            Some(s) => s,
            None => return,
        };

        // Skip plants that have nothing left to do — fully mature with no
        // further stages.  This is ~99 % of trees in a mature forest, so
        // short-circuiting here gets us back the whole plant-update budget.
        if plant.stage_index + 1 >= species.growth_stages.len() {
            return;
        }

        let current_season = self.current_climate
            .as_ref()
            .map(|c| c.season)
            .unwrap_or(Season::Spring);

        // Check dormancy
        let should_be_dormant = species.dormant_in_winter
            && current_season == Season::Winter;

        // Get current stage config
        let stage_config = species.growth_stages.get(plant.stage_index);
        if stage_config.is_none() {
            return;
        }
        let stage_config = stage_config.unwrap();

        // Calculate growth for this tick
        let growth_rate = if should_be_dormant {
            0.0
        } else if species.growing_seasons.contains(&current_season) {
            self.current_climate.as_ref().map(|c| c.growth_rate).unwrap_or(0.5)
        } else {
            0.1 // Minimal growth outside growing season
        };

        // Time needed for current stage
        let stage_duration = species.total_growth_time * stage_config.duration_ratio;
        let progress_per_second = if stage_duration > 0.0 {
            1.0 / stage_duration
        } else {
            1.0
        };

        let progress_delta = progress_per_second * delta_seconds * growth_rate;

        // Apply growth (need mutable borrow now)
        let plant = self.plants.get_mut(plant_id).unwrap();
        plant.is_dormant = should_be_dormant;

        if should_be_dormant {
            return;
        }

        plant.stage_progress += progress_delta;

        // Check stage transition
        if plant.stage_progress >= 1.0 {
            plant.stage_progress = 0.0;
            plant.stage_index += 1;

            if plant.stage_index < species.growth_stages.len() {
                let new_stage = species.growth_stages[plant.stage_index].stage;
                plant.current_stage = new_stage;

                self.pending_events.push(WildlifeEvent::PlantGrow {
                    plant_id: plant.id.clone(),
                    new_stage,
                });

                tracing::debug!(
                    "Plant {} ({}) grew to {:?}",
                    plant.id,
                    plant.species_id,
                    new_stage
                );
            } else {
                // Plant has completed growth cycle
                // For perennials (trees), check if it should wither then regrow
                if species.regrows_after_harvest && species.plant_type == PlantType::FruitTree {
                    // Trees cycle back to mature stage
                    plant.current_stage = species.regrow_stage;
                    plant.stage_index = species
                        .growth_stages
                        .iter()
                        .position(|s| s.stage == species.regrow_stage)
                        .unwrap_or(0);
                } else {
                    // Annual plants wither
                    plant.current_stage = PlantGrowthStage::Withering;
                    self.pending_events.push(WildlifeEvent::PlantGrow {
                        plant_id: plant.id.clone(),
                        new_stage: PlantGrowthStage::Withering,
                    });
                }
            }
        }
    }

    fn check_plant_spawns(&mut self, now_ms: i64, rng: &mut impl Rng) {
        // Target plant counts scaled to zone area (densities per km²).
        let area = self.zone_area_km2();

        let target_ground = ((100.0 * area).round() as usize).max(60);   // grass + clover
        let target_veggies = ((20.0 * area).round() as usize).max(15);   // carrot, potato, onion, garlic
        let target_herbs   = ((16.0 * area).round() as usize).max(12);   // herb_sage, mushroom, berry_bush
        let target_trees   = ((12.0 * area).round() as usize).max(6);    // apple_tree, pear_tree

        // Batch size also scales — don't drip-feed one plant at a time on a 6 km² map
        let batch = ((4.0 * area).round() as usize).clamp(1, 20);

        let ground_count = self
            .plants
            .values()
            .filter(|p| p.is_alive && matches!(p.species_id.as_str(), "grass" | "clover"))
            .count();

        let veggie_count = self
            .plants
            .values()
            .filter(|p| {
                p.is_alive
                    && matches!(
                        p.species_id.as_str(),
                        "carrot" | "potato" | "onion" | "garlic"
                    )
            })
            .count();

        let herb_count = self
            .plants
            .values()
            .filter(|p| {
                p.is_alive
                    && matches!(
                        p.species_id.as_str(),
                        "herb_sage" | "mushroom" | "berry_bush"
                    )
            })
            .count();

        let tree_count = self
            .plants
            .values()
            .filter(|p| {
                p.is_alive && matches!(p.species_id.as_str(), "apple_tree" | "pear_tree")
            })
            .count();

        // Spawn ground cover (grass + clover)
        if ground_count < target_ground {
            let to_spawn = (target_ground - ground_count).min(batch * 2);
            for _ in 0..to_spawn {
                let species = if rng.gen_bool(0.55) { "grass" } else { "clover" };
                self.spawn_plant(species, now_ms, rng);
            }
        }

        // Spawn vegetables
        if veggie_count < target_veggies {
            let veggies = ["carrot", "potato", "onion", "garlic"];
            let to_spawn = (target_veggies - veggie_count).min(batch);
            for _ in 0..to_spawn {
                let species = veggies[rng.gen_range(0..veggies.len())];
                self.spawn_plant(species, now_ms, rng);
            }
        }

        // Spawn herbs/bushes/mushrooms
        if herb_count < target_herbs {
            let herbs = ["herb_sage", "mushroom", "berry_bush"];
            let to_spawn = (target_herbs - herb_count).min(batch);
            for _ in 0..to_spawn {
                let species = herbs[rng.gen_range(0..herbs.len())];
                self.spawn_plant(species, now_ms, rng);
            }
        }

        // Spawn trees
        if tree_count < target_trees {
            let trees = ["apple_tree", "pear_tree"];
            let to_spawn = (target_trees - tree_count).min(batch);
            for _ in 0..to_spawn {
                let species = trees[rng.gen_range(0..trees.len())];
                self.spawn_plant(species, now_ms, rng);
            }
        }
    }

    fn spawn_plant(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng) -> Option<String> {
        self.spawn_plant_inner(species_id, now_ms, rng, false, false)
    }

    fn spawn_plant_forest(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng) -> Option<String> {
        self.spawn_plant_inner(species_id, now_ms, rng, true, false)
    }

    fn spawn_plant_mature(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng) -> Option<String> {
        self.spawn_plant_inner(species_id, now_ms, rng, false, true)
    }

    fn spawn_plant_forest_mature(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng) -> Option<String> {
        self.spawn_plant_inner(species_id, now_ms, rng, true, true)
    }

    /// Spawn a single tree. Picks a candidate position, applies physical
    /// masks, then samples a species from `tree_species_by_biome[biome_at(pos)]`.
    /// `forest_only=true` further constrains positions to OSM forest polygons.
    /// Returns the spawned plant id, or None if no valid position was found
    /// in the attempt budget.
    ///
    /// Replaces the old "fixed species → reject by preferred_biomes" model:
    /// the candidate position is now position-first, species-second, so
    /// geography drives composition (Mountain → pine, Coastal → maple, etc).
    fn spawn_tree_inner(&mut self, now_ms: i64, rng: &mut impl Rng, forest_only: bool) -> Option<String> {
        let max_attempts = if forest_only { 300 } else { 100 };

        // Phase 1: find a valid (position, species) pair. Hold only immutable
        // borrows so the BiomeTreeTable lookup doesn't fight the &mut self
        // we'll need in phase 2 for plant insertion.
        let chosen: Option<(Vector3, String)> = {
            let terrain = self.terrain.as_ref()?;
            let mut found = None;
            for _ in 0..max_attempts {
                let x = rng.gen_range(self.bounds_min.x..self.bounds_max.x);
                let z = rng.gen_range(self.bounds_min.z..self.bounds_max.z);

                if !terrain.can_spawn_flora(x, z, false) { continue; }
                if !terrain.clear_of_structures(x, z, self.tree_structure_clearance) { continue; }
                if !terrain.clear_of_roads(x, z, self.tree_road_clearance) { continue; }
                if !terrain.clear_of_water(x, z, self.tree_water_clearance) { continue; }
                if !self.civic_map.clear_for_tree(x, z) { continue; }

                let in_forest = !self.forest_map.is_empty() && self.forest_map.contains(x, z);
                if forest_only && !in_forest { continue; }

                let cell_biome = terrain_biome_to_zone_biome(terrain.get_biome_at(x, z));
                let species_id = match self.tree_species_by_biome.get(&cell_biome) {
                    Some(table) => match table.pick(rng) {
                        Some(s) => s.to_string(),
                        None => continue,
                    },
                    None => continue, // biome has no entry — no trees here
                };

                let y = terrain.get_elevation(x, z) as f64;
                found = Some((Vector3::new(x, y, z), species_id));
                break;
            }
            found
        };

        let (pos, species_id) = chosen?;
        self.insert_tree_at(&species_id, pos, now_ms)
    }

    /// Insert a mature tree at a known-valid position. Used by `spawn_tree_inner`
    /// after position + species have been picked; not for general use.
    fn insert_tree_at(&mut self, species_id: &str, pos: Vector3, now_ms: i64) -> Option<String> {
        let species = get_plant_species(species_id)?;

        let mature_idx = species.growth_stages.iter()
            .position(|s| s.stage == PlantGrowthStage::Mature)
            .unwrap_or_else(|| species.growth_stages.len().saturating_sub(1));
        let stage = species.growth_stages.get(mature_idx)
            .map(|s| s.stage)
            .unwrap_or(PlantGrowthStage::Mature);

        let variant = (self.next_plant_id % 5) as u8;
        let id = format!("plant_{}_{}", species_id, self.next_plant_id);
        self.next_plant_id += 1;

        let plant = PlantEntity {
            id: id.clone(),
            species_id: species_id.to_string(),
            position: pos,
            zone_id: self.zone_id.clone(),
            current_stage: stage,
            stage_started_at: now_ms,
            stage_progress: 1.0,
            stage_index: mature_idx,
            is_alive: true,
            is_dormant: false,
            times_harvested: 0,
            last_harvested_at: None,
            spawned_at: now_ms,
            variant,
        };

        self.plants.insert(id.clone(), plant);
        self.plant_grid_dirty = true;

        self.pending_events.push(WildlifeEvent::PlantSpawn {
            plant_id: id.clone(),
            species_id: species_id.to_string(),
            position: pos,
            zone_id: self.zone_id.clone(),
            stage,
            variant,
        });

        Some(id)
    }

    fn spawn_plant_inner(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng, forest_only: bool, start_mature: bool) -> Option<String> {
        let species = get_plant_species(species_id)?;

        // Find a valid position (respects terrain biome + structure rules)
        let is_ground_cover = matches!(species_id, "grass" | "clover");
        let pos = if let Some(ref terrain) = self.terrain {
            let mut found = None;
            // Forest-only passes need more attempts since they reject out-of-polygon positions.
            let max_attempts = if forest_only { 300 } else { 100 };
            for _ in 0..max_attempts {
                let x = rng.gen_range(self.bounds_min.x..self.bounds_max.x);
                let z = rng.gen_range(self.bounds_min.z..self.bounds_max.z);

                // Check structure/water/road rules
                if !terrain.can_spawn_flora(x, z, is_ground_cover) {
                    continue;
                }

                let is_tree = matches!(species.plant_type, PlantType::Tree);

                // Trees need clearance from buildings, roads, and water bodies.
                // Use exact OSM geometry when loaded, navmesh flags as fallback.
                if is_tree {
                    if !terrain.clear_of_structures(x, z, self.tree_structure_clearance) { continue; }
                    if !terrain.clear_of_roads(x, z, self.tree_road_clearance) { continue; }
                    if !terrain.clear_of_water(x, z, self.tree_water_clearance) { continue; }
                    if !self.civic_map.clear_for_tree(x, z) { continue; }
                }

                // Forest-only pass: reject positions outside forest polygons.
                let in_forest = is_tree && !self.forest_map.is_empty() && self.forest_map.contains(x, z);
                if forest_only && !in_forest { continue; }

                // Non-trees outside forest polygons must satisfy the species'
                // preferred_biomes filter — keeps grass in grasslands, etc.
                // Trees take a separate code path (`spawn_tree_inner`) that
                // picks species from the cell's biome instead, so they don't
                // hit this branch during initial spawn. Organic respawn for
                // trees doesn't go through this function either.
                if !in_forest && !is_tree {
                    let cell_biome = terrain_biome_to_zone_biome(terrain.get_biome_at(x, z));
                    if !species.preferred_biomes.contains(&cell_biome) {
                        continue;
                    }
                }

                let y = terrain.get_elevation(x, z) as f64;
                found = Some(Vector3::new(x, y, z));
                break;
            }
            found?
        } else {
            // No terrain — fall back to zone-level biome check
            if !species.preferred_biomes.contains(&self.biome) {
                return None;
            }
            Vector3::new(
                rng.gen_range(self.bounds_min.x..self.bounds_max.x),
                0.0,
                rng.gen_range(self.bounds_min.z..self.bounds_max.z),
            )
        };

        let variant = (self.next_plant_id % 5) as u8;
        let id = format!("plant_{}_{}", species_id, self.next_plant_id);
        self.next_plant_id += 1;

        let (initial_stage, stage_index, stage_progress) = if start_mature {
            let idx = species.growth_stages.iter()
                .position(|s| s.stage == PlantGrowthStage::Mature)
                .unwrap_or_else(|| species.growth_stages.len().saturating_sub(1));
            let stage = species.growth_stages.get(idx)
                .map(|s| s.stage)
                .unwrap_or(PlantGrowthStage::Mature);
            (stage, idx, 1.0f64)
        } else {
            let stage = species.growth_stages.first()
                .map(|s| s.stage)
                .unwrap_or(PlantGrowthStage::Seed);
            (stage, 0usize, 0.0f64)
        };

        let plant = PlantEntity {
            id: id.clone(),
            species_id: species_id.to_string(),
            position: pos,
            zone_id: self.zone_id.clone(),

            current_stage: initial_stage,
            stage_started_at: now_ms,
            stage_progress,
            stage_index,

            is_alive: true,
            is_dormant: false,
            times_harvested: 0,
            last_harvested_at: None,

            spawned_at: now_ms,
            variant,
        };

        self.plants.insert(id.clone(), plant);
        self.plant_grid_dirty = true;

        // Notify the game server so clients can see the plant.
        self.pending_events.push(WildlifeEvent::PlantSpawn {
            plant_id: id.clone(),
            species_id: species_id.to_string(),
            position: pos,
            zone_id: self.zone_id.clone(),
            stage: initial_stage,
            variant,
        });

        Some(id)
    }

    /// Spawn initial plants for the zone, scaled to zone area.
    ///
    /// Densities are per km².  The 200 m fallback zone (~0.04 km²) gets
    /// the `min` values; a 2500 m tile (~6.25 km²) gets ≈150× more.
    /// Spawn all flora (trees + plants) from scratch and return positions for caching.
    pub fn spawn_initial_flora(&mut self, now_ms: i64) -> Vec<CachedPlant> {
        let mut rng = rand::thread_rng();
        let area = self.zone_area_km2();
        let forest_area = self.forest_map.area_km2();

        // Trees — two passes, both with biome-driven species selection.
        //
        // Pass 1 (zone-wide): zone_density_per_km² × zone_area_km² candidate
        //   positions scattered uniformly across the whole zone. Physical
        //   masks (water/structures/roads/civic) reject; biome filter does
        //   NOT — the species at each position is picked from
        //   tree_species_by_biome[biome_at(pos)] instead.
        //
        // Pass 2 (forest-extra): forest_density_per_km² × forest_polygon_area_km²
        //   additional positions, each constrained to be inside an OSM forest
        //   polygon. Same biome-driven species pick — so a Mountain forest
        //   polygon comes out conifer-heavy, a Coastal one maple-heavy.
        //
        // All initial-generation plants start mature so the zone is immediately
        // full-grown; seed→mature growth applies only to organic respawns.
        let zone_count   = (self.tree_zone_density_per_km2   * area).round() as usize;
        let forest_count = (self.tree_forest_density_per_km2 * forest_area).round() as usize;

        let mut zone_spawned   = 0usize;
        let mut forest_spawned = 0usize;
        for _ in 0..zone_count {
            if self.spawn_tree_inner(now_ms, &mut rng, false).is_some() {
                zone_spawned += 1;
            }
        }
        for _ in 0..forest_count {
            if self.spawn_tree_inner(now_ms, &mut rng, true).is_some() {
                forest_spawned += 1;
            }
        }

        // Non-tree flora — per-km² densities, biome filter still applies via
        // species.preferred_biomes (handled by spawn_plant_inner).
        let other_specs: &[(&str, f64, usize)] = &[
            // Ground cover
            ("grass",      300.0,  180),
            ("clover",     150.0,   90),
            // Vegetables
            ("carrot",      24.0,   30),
            ("potato",      18.0,   18),
            ("onion",       18.0,   18),
            ("garlic",      12.0,   12),
            // Herbs / mushrooms / berries
            ("herb_sage",   24.0,   24),
            ("mushroom",    30.0,   30),
            ("berry_bush",  24.0,   24),
            // Fruit trees
            ("apple_tree",  25.0,   15),
            ("pear_tree",   20.0,   15),
        ];
        for &(species, density, minimum) in other_specs {
            let count = ((density * area).round() as usize).max(minimum);
            for _ in 0..count {
                self.spawn_plant_mature(species, now_ms, &mut rng);
            }
        }

        tracing::info!(
            "Spawned {} flora in zone {} ({:.2} km², forest {:.2} km²): trees zone={}/{}, forest={}/{}",
            self.plants.len(),
            self.zone_id,
            area,
            forest_area,
            zone_spawned,   zone_count,
            forest_spawned, forest_count,
        );

        self.plants.values()
            .filter(|p| p.is_alive)
            .map(|p| CachedPlant {
                species: p.species_id.clone(),
                x: p.position.x,
                y: p.position.y,
                z: p.position.z,
                variant: p.variant,
            })
            .collect()
    }

    /// Restore all flora from a pre-computed cache at Mature stage.
    pub fn load_cached_flora(&mut self, plants: Vec<CachedPlant>, now_ms: i64) {
        let count = plants.len();
        for plant in plants {
            let species = match get_plant_species(&plant.species) {
                Some(s) => s,
                None => continue,
            };

            let mature_idx = species.growth_stages.iter()
                .position(|s| s.stage == PlantGrowthStage::Mature)
                .unwrap_or_else(|| species.growth_stages.len().saturating_sub(1));
            let stage = species.growth_stages.get(mature_idx)
                .map(|s| s.stage)
                .unwrap_or(PlantGrowthStage::Mature);

            let id = format!("plant_{}_{}", plant.species, self.next_plant_id);
            self.next_plant_id += 1;

            let pos = Vector3::new(plant.x, plant.y, plant.z);
            self.plants.insert(id.clone(), PlantEntity {
                id: id.clone(),
                species_id: plant.species.clone(),
                position: pos,
                zone_id: self.zone_id.clone(),
                current_stage: stage,
                stage_started_at: now_ms,
                stage_progress: 1.0,
                stage_index: mature_idx,
                is_alive: true,
                is_dormant: false,
                times_harvested: 0,
                last_harvested_at: None,
                spawned_at: now_ms,
                variant: plant.variant,
            });
            self.plant_grid_dirty = true;
            self.pending_events.push(WildlifeEvent::PlantSpawn {
                plant_id: id,
                species_id: plant.species,
                position: pos,
                zone_id: self.zone_id.clone(),
                stage,
                variant: plant.variant,
            });
        }
        tracing::info!(
            "Loaded {} cached flora for zone {}",
            count,
            self.zone_id
        );
    }
}
