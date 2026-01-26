//! Climate Simulation Microservice
//!
//! Manages world time, seasons, day/night cycles, and temperature for all zones.
//! Publishes climate state to Redis for other services to consume.
//!
//! Runs at 1Hz (1 update per second) - climate changes slowly!

use anyhow::Result;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{info, error};

mod types;
use types::{Climate, Season, ZoneConfig};

/// Snapshot of climate state published to Redis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateSnapshot {
    pub zone_id: String,
    pub day_of_year: u16,
    pub time_of_day: f64,
    pub year: u32,
    pub season: Season,
    pub temperature: f64,      // -1.0 to 1.0
    pub day_length: f64,       // Hours of daylight
    pub is_night: bool,
    pub growth_rate: f64,      // Combined season + day length modifier
    pub timestamp: u64,        // Unix timestamp ms
}

impl ClimateSnapshot {
    fn from_climate(climate: &Climate, zone_id: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            zone_id: zone_id.to_string(),
            day_of_year: climate.day_of_year,
            time_of_day: climate.time_of_day,
            year: climate.year,
            season: climate.season(),
            temperature: climate.temperature(),
            day_length: climate.day_length(),
            is_night: climate.is_night(),
            growth_rate: climate.growth_rate(),
            timestamp: now,
        }
    }
}

/// Engine that manages climate for multiple zones
pub struct ClimateEngine {
    zones: Arc<RwLock<HashMap<String, Climate>>>,
    configs: HashMap<String, ZoneConfig>,
    redis: ConnectionManager,
}

impl ClimateEngine {
    /// Create new engine with Redis connection
    pub async fn new(redis_url: &str, configs: Vec<ZoneConfig>) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let redis = ConnectionManager::new(client).await?;

        let mut zones = HashMap::new();
        let mut config_map = HashMap::new();

        for config in configs {
            let climate = Climate::new(
                config.starting_day,
                config.starting_time,
                config.latitude,
                config.time_scale,
            );
            zones.insert(config.zone_id.clone(), climate);
            config_map.insert(config.zone_id.clone(), config);
        }

        Ok(Self {
            zones: Arc::new(RwLock::new(zones)),
            configs: config_map,
            redis,
        })
    }

    /// Run the main climate tick loop at 1Hz
    pub async fn run(&mut self) -> Result<()> {
        info!("Climate engine starting - tick rate: 1Hz");
        
        let mut ticker = interval(Duration::from_secs(1));
        let mut tick_count = 0u64;

        loop {
            ticker.tick().await;
            tick_count += 1;

            if let Err(e) = self.tick(1.0).await {
                error!("Tick error: {}", e);
            }

            // Log every 60 seconds
            if tick_count % 60 == 0 {
                self.log_status().await;
            }
        }
    }

    /// Advance all climates and publish to Redis
    async fn tick(&mut self, delta_seconds: f64) -> Result<()> {
        let mut zones = self.zones.write().await;
        
        for (zone_id, climate) in zones.iter_mut() {
            let old_season = climate.season();
            let old_day = climate.day_of_year;
            
            // Advance time
            climate.advance(delta_seconds);
            
            let new_season = climate.season();
            let new_day = climate.day_of_year;
            
            // Create snapshot
            let snapshot = ClimateSnapshot::from_climate(climate, zone_id);
            
            // Publish to Redis
            let channel = format!("climate:zone:{}", zone_id);
            let payload = serde_json::to_string(&snapshot)?;
            // Publish latest snapshot and cache it for consumers
            self.redis.clone().publish::<_, _, ()>(&channel, &payload).await?;
            let key = format!("climate:snapshot:{}", zone_id);
            let _: () = self.redis.clone().set(key, &payload).await?;
            
            // Detect and publish season changes
            if old_season != new_season {
                info!("Zone {} season changed: {:?} -> {:?}", zone_id, old_season, new_season);
                let event = serde_json::json!({
                    "event_type": "season_change",
                    "zone_id": zone_id,
                    "old_season": old_season,
                    "new_season": new_season,
                    "day_of_year": new_day,
                    "timestamp": snapshot.timestamp,
                });
                let event_channel = format!("climate:events:{}", zone_id);
                self.redis.clone().publish::<_, _, ()>(&event_channel, event.to_string()).await?;
            }
            
            // Detect day changes
            if old_day != new_day {
                info!("Zone {} advanced to day {}, Year {}", zone_id, new_day, climate.year);
            }
        }
        
        Ok(())
    }

    /// Log current status of all zones
    async fn log_status(&self) {
        let zones = self.zones.read().await;
        for (zone_id, climate) in zones.iter() {
            info!(
                "Zone {}: {} | Temp: {:.2} | Day length: {:.1}h",
                zone_id,
                climate.format(),
                climate.temperature(),
                climate.day_length()
            );
        }
    }

    /// Get current climate for a zone (for REST API / initial sync)
    pub async fn get_climate(&self, zone_id: &str) -> Option<ClimateSnapshot> {
        let zones = self.zones.read().await;
        zones.get(zone_id).map(|c| ClimateSnapshot::from_climate(c, zone_id))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("climate_sim=info,redis=warn")
        .init();

    info!("Climate Sim v0.1.0 starting...");

    // Configuration - in production this would come from config file or env vars
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let configs = vec![
        ZoneConfig {
            zone_id: "test_zone".to_string(),
            latitude: 42.0,        // NY latitude
            starting_day: 80,      // Day 80 = late March (spring)
            starting_time: 8.0,    // 8 AM
            time_scale: 60.0,      // 1 real second = 1 game minute
        },
        // Add more zones as needed
    ];

    info!("Connecting to Redis at {}", redis_url);
    let mut engine = ClimateEngine::new(&redis_url, configs).await?;

    info!("Climate engine initialized for {} zones", engine.configs.len());
    
    // Run forever
    engine.run().await
}
