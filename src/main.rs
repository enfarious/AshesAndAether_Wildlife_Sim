//! Wildlife Simulation Service
//!
//! A standalone service that simulates wildlife and flora for Ashes & Aether.
//! Connects to the game server via Redis pub/sub.
//!
//! ## Running
//! ```bash
//! cargo run -- --redis redis://localhost:6379 --zone stephentown
//! ```

mod behavior;
mod civic_map;
mod climate;
mod forest_map;
mod climate_subscriber;
mod pathfinding;
mod plant_species;
mod redis_bridge;
mod simulation;
mod species;
mod terrain;
mod terrain_client;
mod types;
mod weather_subscriber;

use anyhow::Result;
use clap::Parser;
use civic_map::CivicMap;
use forest_map::ForestMap;
use serde::Deserialize;
use simulation::{CachedPlant, ZoneSimulation};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use types::*;

/// Wildlife simulation service for Ashes & Aether
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct CliArgs {
    /// Redis connection URL
    #[arg(long)]
    redis: Option<String>,

    /// Zone ID to simulate (or "all" for all zones)
    #[arg(long)]
    zone: Option<String>,

    /// Tick rate in Hz
    #[arg(long)]
    tick_rate: Option<u32>,

    /// Run without the game server message loop
    #[arg(long)]
    offline: bool,

    /// Time scale (game seconds per real second, default 60 = 1 min/sec)
    #[arg(long)]
    time_scale: Option<f64>,

    /// Game server URL for terrain data
    #[arg(long)]
    server_url: Option<String>,

    /// Tile ID to simulate (or "auto" for first available, "none" to skip)
    #[arg(long)]
    tile: Option<String>,

    /// Tile size in world meters (for offline/fallback terrain)
    #[arg(long)]
    tile_size: Option<f64>,

    /// Path to config file
    #[arg(long, default_value = "config.toml")]
    config: String,
}

/// Resolved configuration (config file defaults + CLI overrides).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct FloraConfig {
    zone_tree_multiplier:    f64,
    forest_tree_multiplier:  f64,
    tree_building_clearance: f64,
    tree_road_clearance:     f64,
    tree_water_clearance:    f64,
}

impl Default for FloraConfig {
    fn default() -> Self {
        Self {
            zone_tree_multiplier:    3.0,
            forest_tree_multiplier:  10.0,
            tree_building_clearance: 20.0,
            tree_road_clearance:     10.0,
            tree_water_clearance:    5.0,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct Config {
    redis: String,
    zone: String,
    tick_rate: u32,
    offline: bool,
    time_scale: f64,
    server_url: String,
    tile: String,
    tile_size: f64,
    flora: FloraConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            redis: "redis://127.0.0.1:6379".into(),
            zone: "all".into(),
            tick_rate: 10,
            offline: false,
            time_scale: 60.0,
            server_url: "http://127.0.0.1:3100".into(),
            tile: "auto".into(),
            tile_size: 200.0,
            flora: FloraConfig::default(),
        }
    }
}

impl Config {
    /// Load from TOML file, then override with any explicit CLI flags.
    fn load(cli: &CliArgs) -> Self {
        // Try to read the config file
        let mut cfg = if std::path::Path::new(&cli.config).exists() {
            match config::Config::builder()
                .add_source(config::File::with_name(&cli.config))
                .build()
            {
                Ok(settings) => match settings.try_deserialize::<Config>() {
                    Ok(c) => {
                        info!("Loaded config from {}", cli.config);
                        c
                    }
                    Err(e) => {
                        warn!("Failed to parse {}: {}. Using defaults.", cli.config, e);
                        Config::default()
                    }
                },
                Err(e) => {
                    warn!("Failed to read {}: {}. Using defaults.", cli.config, e);
                    Config::default()
                }
            }
        } else {
            info!("No config file found at {}. Using defaults.", cli.config);
            Config::default()
        };

        // CLI flags override config file values
        if let Some(ref v) = cli.redis      { cfg.redis = v.clone(); }
        if let Some(ref v) = cli.zone       { cfg.zone = v.clone(); }
        if let Some(v)     = cli.tick_rate   { cfg.tick_rate = v; }
        if cli.offline                       { cfg.offline = true; }
        if let Some(v)     = cli.time_scale  { cfg.time_scale = v; }
        if let Some(ref v) = cli.server_url  { cfg.server_url = v.clone(); }
        if let Some(ref v) = cli.tile        { cfg.tile = v.clone(); }
        if let Some(v)     = cli.tile_size   { cfg.tile_size = v; }

        cfg
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("wildlife_sim=info".parse()?)
                .add_directive("warn".parse()?),
        )
        .init();

    let cli = CliArgs::parse();
    let args = Config::load(&cli);

    info!("Starting Wildlife Simulation Service");
    info!("  Redis: {}", args.redis);
    info!("  Zone: {}", args.zone);
    info!("  Tick rate: {} Hz", args.tick_rate);
    info!("  Time scale: {}x (1 sec = {} game sec)", args.time_scale, args.time_scale);
    info!("  Server URL: {}", args.server_url);
    info!("  Tile: {}", args.tile);

    if args.offline {
        info!("Running in OFFLINE mode (no Redis)");
        run_offline(args).await
    } else {
        run_with_redis(args).await
    }
}

async fn run_with_redis(args: Config) -> Result<()> {
    let mut bridge = redis_bridge::RedisBridge::connect(&args.redis).await?;

    // Zone simulations - will be populated from game server messages
    let mut zones: HashMap<String, ZoneSimulation> = HashMap::new();

    // Load terrain first so it can be applied to zones as they're created
    let terrain = load_terrain(&args).await;

    // Try to prime zones from snapshot keys left by the game server.
    // This avoids the race condition where zone info was published before we connected.
    // handle_game_message now applies terrain and spawns populations automatically.
    let snapshots = bridge.fetch_zone_snapshots(&args.zone).await;
    if snapshots.is_empty() {
        info!("No zone snapshots found — waiting for server to publish zone info...");
    } else {
        info!("Found {} zone snapshot(s) — priming zones", snapshots.len());
        for msg in snapshots {
            handle_game_message(&mut zones, msg, &args.redis, &args.server_url, &terrain, args.time_scale, &args.flora).await;
        }
    }

    let tick_duration = Duration::from_secs_f64(1.0 / args.tick_rate as f64);
    let mut last_tick = Instant::now();
    let mut last_reannounce = Instant::now();
    let reannounce_interval = Duration::from_secs(15);

    info!("Entering main loop ({} zone(s) active)...", zones.len());

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_tick);

        if elapsed >= tick_duration {
            let now_ms = chrono::Utc::now().timestamp_millis();
            // Cap at 2× the target tick interval so a slow tick can't produce
            // a giant movement step and visible animal teleport. The sim still
            // advances time, just at a bounded rate when overloaded.
            let max_delta = (tick_duration.as_secs_f64() * 2.0).min(0.25);
            let delta_seconds = elapsed.as_secs_f64().min(max_delta);

            // Process incoming messages
            while let Some(msg) = bridge.try_recv() {
                handle_game_message(&mut zones, msg, &args.redis, &args.server_url, &terrain, args.time_scale, &args.flora).await;
            }

            // Periodically re-announce all entities so that late-connecting
            // or restarted game servers can rebuild their entity registry.
            if now.duration_since(last_reannounce) >= reannounce_interval {
                for zone in zones.values_mut() {
                    zone.re_announce_all();
                }
                last_reannounce = now;
            }

            // Update all zones
            let tick_start = Instant::now();
            for zone in zones.values_mut() {
                zone.update(now_ms, delta_seconds).await;

                // Publish events
                let events = zone.take_events();
                if !events.is_empty() {
                    if let Err(e) = bridge.publish_events(&zone.zone_id, events).await {
                        warn!("Failed to publish events: {}", e);
                    }
                }
            }

            // Warn if a tick took >2× the budget — diagnoses sim overload.
            let tick_elapsed = tick_start.elapsed();
            if tick_elapsed > tick_duration * 2 {
                warn!(
                    "Tick over budget: {} ms (target {} ms)",
                    tick_elapsed.as_millis(),
                    tick_duration.as_millis(),
                );
            }

            last_tick = now;
        }

        // Small sleep to prevent busy-waiting
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn run_offline(args: Config) -> Result<()> {
    // Initialize terrain
    let terrain = load_terrain(&args).await;

    let (bounds_min, bounds_max) = terrain
        .as_ref()
        .map(|t| t.world_bounds())
        .unwrap_or((
            Vector3::new(-args.tile_size / 2.0, 0.0, -args.tile_size / 2.0),
            Vector3::new(args.tile_size / 2.0, 0.0, args.tile_size / 2.0),
        ));

    // Create the zone (use config zone ID so Redis channels match the server)
    let zone_id = if args.zone == "all" { "test_zone".to_string() } else { args.zone.clone() };
    info!("  Zone ID: {}", zone_id);
    let mut zone = ZoneSimulation::new(
        zone_id,
        BiomeType::Grassland,
        bounds_min,
        bounds_max,
    );

    if let Some(t) = terrain {
        zone.set_terrain(t);
        info!("Terrain loaded — water sources, walkability, and elevation active");
    } else {
        // Fallback: hardcoded water sources (no terrain)
        zone.water_sources = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(50.0, 0.0, 50.0),
            Vector3::new(-50.0, 0.0, -50.0),
        ];
    }

    // Set internal climate time scale to match configuration
    zone.set_time_scale(args.time_scale);
    zone.set_flora_config(&args.flora);

    // Offline mode still uses Redis for climate_sim and for publishing
    // wildlife events so the game server bridge can pick them up.
    info!("Initializing Redis connection for climate + event publishing...");
    let mut bridge = redis_bridge::RedisBridge::connect(&args.redis).await?;
    zone.init_climate(&args.redis).await?;
    zone.init_weather(&args.redis).await?;
    info!("Redis initialized (climate + event publishing)");

    // Spawn initial populations (density-scaled to zone area)
    let now_ms = chrono::Utc::now().timestamp_millis();
    info!("Zone area: {:.2} km² — scaling populations to match", zone.zone_area_km2());
    for (species, males, females) in density_scaled_populations(&zone) {
        let count = zone.spawn_population(species, males, females, now_ms);
        info!("  Spawned {} {} ({}M + {}F)", count, species, males, females);
    }

    // Load forest polygons so trees are restricted to OSM wood/forest areas
    let zone_id_for_forest = if args.zone == "all" { "test_zone".to_string() } else { args.zone.clone() };
    let forest = ForestMap::fetch(&args.server_url, &zone_id_for_forest).await;
    zone.set_forest_map(forest);
    let civic = CivicMap::fetch(&args.server_url, &zone_id_for_forest).await;
    zone.set_civic_map(civic);

    // Flora — load from cache or generate and save.
    if let Some(cached) = load_flora_cache(&zone_id_for_forest) {
        zone.load_cached_flora(cached, now_ms);
    } else {
        let flora = zone.spawn_initial_flora(now_ms);
        save_flora_cache(&zone_id_for_forest, &flora);
    }

    let tick_duration = Duration::from_secs_f64(1.0 / args.tick_rate as f64);
    let mut last_tick = Instant::now();
    let mut last_reannounce = Instant::now();
    let reannounce_interval = Duration::from_secs(15);

    info!("Running offline simulation...");

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_tick);

        if elapsed >= tick_duration {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let max_delta = (tick_duration.as_secs_f64() * 2.0).min(0.25);
            let delta_seconds = elapsed.as_secs_f64().min(max_delta);

            // Periodically re-announce all entities so that late-connecting
            // or restarted game servers can rebuild their entity registry.
            if now.duration_since(last_reannounce) >= reannounce_interval {
                zone.re_announce_all();
                last_reannounce = now;
            }

            zone.update(now_ms, delta_seconds).await;

            // Publish events to Redis so the game server bridge can receive them
            let events = zone.take_events();
            if !events.is_empty() {
                // Log non-move events
                for event in &events {
                    match event {
                        crate::types::WildlifeEvent::Move { .. } => {} // too noisy
                        _ => info!("Event: {:?}", event),
                    }
                }
                if let Err(e) = bridge.publish_events(&zone.zone_id, events).await {
                    warn!("Failed to publish events: {}", e);
                }
            }

            last_tick = now;
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

// ── Tree position cache ───────────────────────────────────────────────────────

fn flora_cache_path(zone_id: &str) -> PathBuf {
    // v5: initial flora spawns at mature stage instead of seed stage.
    PathBuf::from(format!("data/{}/flora_v5.json", zone_id))
}

fn load_flora_cache(zone_id: &str) -> Option<Vec<CachedPlant>> {
    let path = flora_cache_path(zone_id);
    if !path.exists() { return None; }
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<Vec<CachedPlant>>(&s) {
            Ok(v) => {
                info!("Loaded {} cached flora for zone {} from {}", v.len(), zone_id, path.display());
                Some(v)
            }
            Err(e) => {
                warn!("Failed to parse flora cache {}: {} — regenerating", path.display(), e);
                None
            }
        },
        Err(e) => {
            warn!("Failed to read flora cache {}: {} — regenerating", path.display(), e);
            None
        }
    }
}

fn save_flora_cache(zone_id: &str, flora: &[CachedPlant]) {
    let path = flora_cache_path(zone_id);
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("Failed to create cache dir {}: {}", dir.display(), e);
            return;
        }
    }
    match serde_json::to_string(flora) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                warn!("Failed to write flora cache {}: {}", path.display(), e);
            } else {
                info!("Saved {} flora positions to {}", flora.len(), path.display());
            }
        }
        Err(e) => warn!("Failed to serialize flora cache: {}", e),
    }
}

/// Try to load terrain from the server with retries, falling back to procedural generation.
async fn load_terrain(args: &Config) -> Option<terrain::TerrainGrid> {
    if args.tile == "none" {
        info!("Terrain disabled (--tile none)");
        return None;
    }

    let origin = Vector3::new(-args.tile_size / 2.0, 0.0, -args.tile_size / 2.0);
    let client = terrain_client::TerrainClient::new(&args.server_url);

    // Retry up to 5 times with increasing backoff — the game server's HTTP
    // endpoint may not be ready yet even though Redis is already accepting
    // connections.
    let max_retries = 5;
    let mut last_err = String::new();

    for attempt in 1..=max_retries {
        let result = if args.tile == "auto" {
            match client.fetch_manifest().await {
                Ok(manifest) if !manifest.tiles.is_empty() => {
                    // Prefer a tile whose ID matches the zone being simulated.
                    let tile_id = manifest.tiles
                        .iter()
                        .find(|t| t.id == args.zone)
                        .or_else(|| manifest.tiles.first())
                        .map(|t| t.id.clone())
                        .unwrap();
                    info!("Auto-selected tile: {} (zone: {})", tile_id, args.zone);
                    client
                        .fetch_and_build_grid(&tile_id, args.tile_size, origin)
                        .await
                }
                Ok(_) => Err(anyhow::anyhow!("No tiles available on server")),
                Err(e) => Err(e),
            }
        } else {
            client
                .fetch_and_build_grid(&args.tile, args.tile_size, origin)
                .await
        };

        match result {
            Ok(grid) => {
                info!("Terrain loaded from server (attempt {})", attempt);
                return Some(grid);
            }
            Err(e) => {
                last_err = format!("{}", e);
                if attempt < max_retries {
                    let delay_secs = attempt as u64 * 2; // 2, 4, 6, 8 seconds
                    warn!(
                        "Terrain fetch attempt {}/{} failed: {}. Retrying in {}s...",
                        attempt, max_retries, e, delay_secs
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                }
            }
        }
    }

    warn!(
        "Failed to load terrain after {} attempts: {}. Generating fallback.",
        max_retries, last_err
    );
    Some(terrain_client::generate_fallback_terrain(args.tile_size, origin))
}

/// Compute initial population counts scaled to zone area.
///
/// Returns `[(species, males, females), ...]`.  Densities are per km²;
/// for the 200 m fallback terrain (~0.04 km²) the minimums ensure at
/// least a small starter population.
fn density_scaled_populations(zone: &ZoneSimulation) -> Vec<(&'static str, usize, usize)> {
    let area = zone.zone_area_km2();

    // (species, males/km², females/km², min_males, min_females)
    let specs: &[(&str, f64, f64, usize, usize)] = &[
        ("rabbit", 12.0, 18.0,  9, 12),  // ~30 /km²  — common prey
        ("fox",     2.5,  3.0,  4,  5),  // ~5.5/km²  — solitary predator
        ("deer",    4.5,  6.5,  6,  9),  // ~11 /km²  — herd herbivore
        ("wolf",    1.5,  2.0,  3,  4),  // ~3.5/km²  — pack predator
        ("boar",    2.5,  3.0,  4,  5),  // ~5.5/km²  — omnivore
    ];

    specs
        .iter()
        .map(|&(species, m_den, f_den, min_m, min_f)| {
            let males   = ((m_den * area).round() as usize).max(min_m);
            let females = ((f_den * area).round() as usize).max(min_f);
            (species, males, females)
        })
        .collect()
}

async fn handle_game_message(
    zones: &mut HashMap<String, ZoneSimulation>,
    msg: GameServerMessage,
    redis_url: &str,
    server_url: &str,
    terrain: &Option<terrain::TerrainGrid>,
    time_scale: f64,
    flora: &FloraConfig,
) {
    match msg {
        GameServerMessage::ZoneInfo { zone } => {
            info!("Received zone info: {} ({:?})", zone.id, zone.biome);

            if let Some(existing) = zones.get_mut(&zone.id) {
                // Zone already exists — the game server likely restarted.
                // Re-announce every living entity so the bridge rebuilds
                // its entity registry.
                info!("Zone {} already known — re-announcing entities", zone.id);
                existing.re_announce_all();
            } else {
                let mut sim = ZoneSimulation::new(zone.id.clone(), zone.biome, zone.bounds_min, zone.bounds_max);
                sim.set_time_scale(time_scale);
                sim.set_flora_config(flora);

                if let Err(e) = sim.init_climate(redis_url).await {
                    warn!("Failed to initialize climate for zone {}: {}", zone.id, e);
                } else {
                    info!("Climate initialized for zone {}", zone.id);
                }
                if let Err(e) = sim.init_weather(redis_url).await {
                    warn!("Failed to init weather: {}", e);
                }

                // Apply terrain if available
                if let Some(t) = terrain {
                    sim.set_terrain(t.clone());
                }

                // Load forest polygons so trees spawn inside OSM wood/forest areas
                let forest = ForestMap::fetch(server_url, &zone.id).await;
                sim.set_forest_map(forest);
                let civic = CivicMap::fetch(server_url, &zone.id).await;
                sim.set_civic_map(civic);

                // Spawn initial populations (density-scaled to zone area)
                let now_ms = chrono::Utc::now().timestamp_millis();
                info!("Zone {}: area {:.2} km² — scaling populations", zone.id, sim.zone_area_km2());
                for (species, males, females) in density_scaled_populations(&sim) {
                    let count = sim.spawn_population(species, males, females, now_ms);
                    info!("  Zone {}: spawned {} {} ({}M + {}F)", zone.id, count, species, males, females);
                }
                // Flora — load from cache or generate and save.
                if let Some(cached) = load_flora_cache(&zone.id) {
                    sim.load_cached_flora(cached, now_ms);
                } else {
                    let flora = sim.spawn_initial_flora(now_ms);
                    save_flora_cache(&zone.id, &flora);
                }

                zones.insert(zone.id, sim);
            }
        }

        GameServerMessage::PlayersUpdate { players } => {
            // Clear ALL zones first so that zones whose players left are
            // correctly emptied (prevents animals fleeing from stale positions).
            for zone in zones.values_mut() {
                zone.update_players(Vec::new());
            }

            // Group players by zone
            let mut by_zone: HashMap<String, Vec<PlayerPosition>> = HashMap::new();
            for player in players {
                by_zone
                    .entry(player.zone_id.clone())
                    .or_default()
                    .push(player);
            }

            for (zone_id, zone_players) in by_zone {
                if let Some(zone) = zones.get_mut(&zone_id) {
                    zone.update_players(zone_players);
                }
            }
        }

        GameServerMessage::PlayerAttack {
            player_id,
            target_id,
            damage,
        } => {
            // Find which zone has this target
            for zone in zones.values_mut() {
                if zone.wildlife.contains_key(&target_id) {
                    zone.player_attacked(&target_id, damage, &player_id);
                    break;
                }
            }
        }

        GameServerMessage::PlantHarvest { plant_id, .. } => {
            // Find and mark plant as harvested
            for zone in zones.values_mut() {
                if let Some(plant) = zone.plants.get_mut(&plant_id) {
                    plant.is_alive = false;
                    break;
                }
            }
        }

        GameServerMessage::StateRequest { zone_id } => {
            info!("State request for zone {}", zone_id);
            // State will be published in the main loop
        }
    }
}
