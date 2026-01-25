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
mod plant_species;
mod redis_bridge;
mod simulation;
mod species;
mod types;

use anyhow::Result;
use clap::Parser;
use simulation::ZoneSimulation;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use types::*;

/// Wildlife simulation service for Ashes & Aether
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Redis connection URL
    #[arg(long, default_value = "redis://127.0.0.1:6379")]
    redis: String,

    /// Zone ID to simulate (or "all" for all zones)
    #[arg(long, default_value = "all")]
    zone: String,

    /// Tick rate in Hz
    #[arg(long, default_value = "10")]
    tick_rate: u32,

    /// Run without Redis (for testing)
    #[arg(long)]
    offline: bool,

    /// Time scale (game seconds per real second, default 60 = 1 min/sec)
    #[arg(long, default_value = "60")]
    time_scale: f64,
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

    let args = Args::parse();

    info!("Starting Wildlife Simulation Service");
    info!("  Redis: {}", args.redis);
    info!("  Zone: {}", args.zone);
    info!("  Tick rate: {} Hz", args.tick_rate);
    info!("  Time scale: {}x (1 sec = {} game sec)", args.time_scale, args.time_scale);

    if args.offline {
        info!("Running in OFFLINE mode (no Redis)");
        run_offline(args).await
    } else {
        run_with_redis(args).await
    }
}

async fn run_with_redis(args: Args) -> Result<()> {
    let mut bridge = redis_bridge::RedisBridge::connect(&args.redis).await?;

    // Zone simulations - will be populated from game server messages
    let mut zones: HashMap<String, ZoneSimulation> = HashMap::new();

    let tick_duration = Duration::from_secs_f64(1.0 / args.tick_rate as f64);
    let mut last_tick = Instant::now();

    info!("Entering main loop...");

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_tick);

        if elapsed >= tick_duration {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let delta_seconds = elapsed.as_secs_f64();

            // Process incoming messages
            while let Some(msg) = bridge.try_recv() {
                handle_game_message(&mut zones, msg);
            }

            // Update all zones
            for zone in zones.values_mut() {
                zone.update(now_ms, delta_seconds);

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

async fn run_offline(args: Args) -> Result<()> {
    // Create climate with configurable time scale
    let climate = climate::Climate::new(
        80,              // Day 80 = late March (early spring)
        8.0,             // 8 AM
        42.0,            // NY latitude
        args.time_scale, // Configurable time scale
    );

    // Create a test zone with climate
    let mut zone = ZoneSimulation::with_climate(
        "test_zone".to_string(),
        BiomeType::Grassland,
        Vector3::new(-100.0, 0.0, -100.0),
        Vector3::new(100.0, 0.0, 100.0),
        climate,
    );

    // Add some water sources
    zone.water_sources = vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(50.0, 0.0, 50.0),
        Vector3::new(-50.0, 0.0, -50.0),
    ];

    // Spawn initial populations: 2 males, 5 females each
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rabbits = zone.spawn_population("rabbit", 2, 5, now_ms);
    let foxes = zone.spawn_population("fox", 2, 5, now_ms);
    info!("Spawned initial population: {} rabbits, {} foxes", rabbits, foxes);

    // Spawn initial plants
    zone.spawn_initial_plants(now_ms);

    let tick_duration = Duration::from_secs_f64(1.0 / args.tick_rate as f64);
    let mut last_tick = Instant::now();
    let mut last_report = Instant::now();

    info!("Running offline simulation...");

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_tick);

        if elapsed >= tick_duration {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let delta_seconds = elapsed.as_secs_f64();

            zone.update(now_ms, delta_seconds);

            // Log significant events only (skip move events)
            let events = zone.take_events();
            for event in &events {
                match event {
                    crate::types::WildlifeEvent::Move { .. } => {} // too noisy
                    _ => info!("Event: {:?}", event),
                }
            }

            last_tick = now;
        }

        // Periodic status report
        if now.duration_since(last_report) > Duration::from_secs(10) {
            let alive_count = zone.wildlife.values().filter(|e| e.is_alive).count();
            let alive_plants = zone.plants.values().filter(|p| p.is_alive).count();

            info!(
                "=== {} | {} wildlife, {} plants ===",
                zone.climate.format(),
                alive_count,
                alive_plants
            );

            for entity in zone.wildlife.values().filter(|e| e.is_alive) {
                info!(
                    "  {} ({}) - {:?} - H:{:.0} T:{:.0} E:{:.0} R:{:.0}",
                    entity.name,
                    entity.species_id,
                    entity.current_behavior,
                    entity.needs.hunger,
                    entity.needs.thirst,
                    entity.needs.energy,
                    entity.needs.reproduction,
                );
            }

            // Show plant summary by stage
            let mut stage_counts: std::collections::HashMap<PlantGrowthStage, usize> =
                std::collections::HashMap::new();
            for plant in zone.plants.values().filter(|p| p.is_alive) {
                *stage_counts.entry(plant.current_stage).or_insert(0) += 1;
            }
            if !stage_counts.is_empty() {
                info!(
                    "  Plants: {:?}",
                    stage_counts
                );
            }

            last_report = now;
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn handle_game_message(zones: &mut HashMap<String, ZoneSimulation>, msg: GameServerMessage) {
    match msg {
        GameServerMessage::ZoneInfo { zone } => {
            info!("Received zone info: {} ({:?})", zone.id, zone.biome);

            let sim = zones
                .entry(zone.id.clone())
                .or_insert_with(|| {
                    ZoneSimulation::new(zone.id, zone.biome, zone.bounds_min, zone.bounds_max)
                });
            sim.climate.time_of_day = zone.time_of_day;
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
