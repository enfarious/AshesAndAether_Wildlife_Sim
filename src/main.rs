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
mod climate;
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
use serde::Deserialize;
use simulation::ZoneSimulation;
use std::collections::HashMap;
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            redis: "redis://127.0.0.1:6379".into(),
            zone: "all".into(),
            tick_rate: 10,
            offline: false,
            time_scale: 60.0,
            server_url: "http://127.0.0.1:3000".into(),
            tile: "auto".into(),
            tile_size: 200.0,
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
                .add_directive("wildlife_sim=debug".parse()?)
                .add_directive("info".parse()?),
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
            handle_game_message(&mut zones, msg, &args.redis, &terrain).await;
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
            let delta_seconds = elapsed.as_secs_f64();

            // Process incoming messages
            while let Some(msg) = bridge.try_recv() {
                handle_game_message(&mut zones, msg, &args.redis, &terrain).await;
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
            for zone in zones.values_mut() {
                zone.update(now_ms, delta_seconds).await;

                // Publish events
                let events = zone.take_events();
                if !events.is_empty() {
                    if let Err(e) = bridge.publish_events(events).await {
                        warn!("Failed to publish events: {}", e);
                    }
                }
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

    // Offline mode still uses Redis for climate_sim and for publishing
    // wildlife events so the game server bridge can pick them up.
    info!("Initializing Redis connection for climate + event publishing...");
    let mut bridge = redis_bridge::RedisBridge::connect(&args.redis).await?;
    zone.init_climate(&args.redis).await?;
    zone.init_weather(&args.redis).await?;
    info!("Redis initialized (climate + event publishing)");

    // Spawn initial populations
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rabbits = zone.spawn_population("rabbit", 3, 7, now_ms);
    let foxes = zone.spawn_population("fox", 2, 4, now_ms);
    let deer = zone.spawn_population("deer", 2, 5, now_ms);
    let wolves = zone.spawn_population("wolf", 1, 3, now_ms);
    let boars = zone.spawn_population("boar", 2, 4, now_ms);
    info!(
        "Spawned initial population: {} rabbits, {} foxes, {} deer, {} wolves, {} boars",
        rabbits, foxes, deer, wolves, boars
    );

    // Spawn initial plants
    zone.spawn_initial_plants(now_ms);

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
            let delta_seconds = elapsed.as_secs_f64();

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
                if let Err(e) = bridge.publish_events(events).await {
                    warn!("Failed to publish events: {}", e);
                }
            }

            last_tick = now;
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Try to load terrain from the server, falling back to procedural generation.
async fn load_terrain(args: &Config) -> Option<terrain::TerrainGrid> {
    if args.tile == "none" {
        info!("Terrain disabled (--tile none)");
        return None;
    }

    let origin = Vector3::new(-args.tile_size / 2.0, 0.0, -args.tile_size / 2.0);

    // Try fetching from server
    let client = terrain_client::TerrainClient::new(&args.server_url);

    let result = if args.tile == "auto" {
        match client.fetch_manifest().await {
            Ok(manifest) if !manifest.tiles.is_empty() => {
                let tile_id = &manifest.tiles[0].id;
                info!("Auto-selected tile: {}", tile_id);
                client
                    .fetch_and_build_grid(tile_id, args.tile_size, origin)
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
            info!("Terrain loaded from server");
            Some(grid)
        }
        Err(e) => {
            warn!("Failed to load terrain from server: {}. Generating fallback.", e);
            Some(terrain_client::generate_fallback_terrain(args.tile_size, origin))
        }
    }
}

async fn handle_game_message(
    zones: &mut HashMap<String, ZoneSimulation>,
    msg: GameServerMessage,
    redis_url: &str,
    terrain: &Option<terrain::TerrainGrid>,
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

                // Spawn initial populations for the new zone
                let now_ms = chrono::Utc::now().timestamp_millis();
                let rabbits = sim.spawn_population("rabbit", 3, 7, now_ms);
                let foxes = sim.spawn_population("fox", 2, 4, now_ms);
                let deer = sim.spawn_population("deer", 2, 5, now_ms);
                let wolves = sim.spawn_population("wolf", 1, 3, now_ms);
                let boars = sim.spawn_population("boar", 2, 4, now_ms);
                info!(
                    "Zone {}: spawned {} rabbits, {} foxes, {} deer, {} wolves, {} boars",
                    zone.id, rabbits, foxes, deer, wolves, boars
                );
                sim.spawn_initial_plants(now_ms);

                zones.insert(zone.id, sim);
            }
        }

        GameServerMessage::PlayersUpdate { players } => {
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
