mod types;

use anyhow::Result;
use rand::Rng;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};
use types::{WeatherConfig, WeatherEvent, WeatherEventType, WeatherSnapshot};

/// Main weather engine managing all zones
pub struct WeatherEngine {
    redis: ConnectionManager,
    zones: HashMap<String, ZoneWeather>,
}

/// Weather state for a single zone
struct ZoneWeather {
    config: WeatherConfig,
    snapshot: WeatherSnapshot,
    current_climate: Option<types::ClimateSnapshot>,
    next_event_check_ms: i64,
}

impl WeatherEngine {
    pub async fn new(redis_url: &str, configs: Vec<WeatherConfig>) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let redis = ConnectionManager::new(client).await?;

        let mut zones = HashMap::new();
        let now_ms = Self::current_time_ms();

        for config in configs {
            let zone_id = config.zone_id.clone();
            let snapshot = WeatherSnapshot::calm(zone_id.clone(), now_ms);
            zones.insert(
                zone_id,
                ZoneWeather {
                    config,
                    snapshot,
                    current_climate: None,
                    next_event_check_ms: now_ms + 10_000, // Check in 10 seconds
                },
            );
        }

        Ok(Self {
            redis,
            zones,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Weather engine starting - tick every 10s (0.1Hz)");
        
        let tick_duration = Duration::from_secs(10); // 0.1Hz for testing
        let mut ticker = interval(tick_duration);

        loop {
            ticker.tick().await;
            if let Err(e) = self.tick().await {
                error!("Tick error: {}", e);
            }
        }
    }

    async fn tick(&mut self) -> Result<()> {
        let now_ms = Self::current_time_ms();
        let mut rng = rand::thread_rng();

        // Collect snapshots to publish (avoid borrow conflict)
        let mut snapshots_to_publish = Vec::new();

        for (zone_id, zone_weather) in self.zones.iter_mut() {
            // Try to get latest climate from Redis (non-blocking)
            if let Ok(climate_json) = self.redis.get::<_, Option<String>>(format!("climate:snapshot:{}", zone_id)).await {
                if let Some(json) = climate_json {
                    if let Ok(climate) = serde_json::from_str(&json) {
                        zone_weather.current_climate = Some(climate);
                    }
                }
            }

            // Update active events (movement, expiry)
            zone_weather.update_events(now_ms);

            // Check for new weather events periodically
            if now_ms >= zone_weather.next_event_check_ms {
                zone_weather.check_spawn_events(now_ms, &mut rng);
                zone_weather.next_event_check_ms = now_ms + 5_000; // Check every 5 seconds
            }

            // Update base weather conditions (wind, clouds)
            zone_weather.update_base_conditions(&mut rng);

            // Update snapshot timestamp
            zone_weather.snapshot.timestamp_ms = now_ms;

            // Clone snapshot for publishing
            snapshots_to_publish.push((zone_id.clone(), zone_weather.snapshot.clone()));
        }

        // Publish all snapshots
        for (zone_id, snapshot) in snapshots_to_publish {
            self.publish_weather(&zone_id, &snapshot).await?;
        }

        Ok(())
    }

    async fn publish_weather(&mut self, zone_id: &str, snapshot: &WeatherSnapshot) -> Result<()> {
        let channel = format!("weather:zone:{}", zone_id);
        let json = serde_json::to_string(snapshot)?;
        
        let _: () = self.redis.publish::<_, _, ()>(&channel, json.clone()).await?;
        let key = format!("weather:snapshot:{}", zone_id);
        let _: () = self.redis.set::<_, _, ()>(&key, json).await?;
        
        debug!("Published weather for zone {}: {} events", zone_id, snapshot.active_events.len());
        Ok(())
    }

    fn current_time_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }
}

impl ZoneWeather {
    fn update_events(&mut self, now_ms: i64) {
        // Remove expired events
        self.snapshot.active_events.retain(|event| {
            let elapsed = now_ms - event.start_time_ms;
            elapsed < event.duration_ms
        });

        // Update moving events (tornadoes)
        for event in &mut self.snapshot.active_events {
            if let WeatherEventType::Tornado { speed, direction, .. } = event.event_type {
                let delta_seconds = 1.0 / 5.0; // Assuming 5Hz update rate
                let dx = direction.cos() * speed * delta_seconds;
                let dz = direction.sin() * speed * delta_seconds;
                
                event.position[0] += dx;
                event.position[2] += dz;

                // Remove if out of bounds
                if event.position[0] < self.config.bounds_min[0] || 
                   event.position[0] > self.config.bounds_max[0] ||
                   event.position[2] < self.config.bounds_min[2] || 
                   event.position[2] > self.config.bounds_max[2] {
                    event.is_active = false;
                }
            }
        }

        // Remove inactive events
        self.snapshot.active_events.retain(|e| e.is_active);
    }

    fn check_spawn_events(&mut self, now_ms: i64, rng: &mut impl Rng) {
        // Get climate-modified probabilities (clone to avoid borrow issues)
        let climate = self.current_climate.clone();
        
        // Convert hourly probabilities to 10-second check probabilities (0.1Hz)
        let check_interval = 10.0; // seconds
        let hour_to_check = check_interval / 3600.0;

        // Climate-aware storm spawning
        let storm_mod = climate.as_ref().map(|c| match c.season {
            types::Season::Spring => 1.5,  // Spring storms common
            types::Season::Summer => 2.0,  // Summer thunderstorms
            types::Season::Fall => 1.2,
            types::Season::Winter => 0.3,  // Rare in winter (that's blizzards)
        }).unwrap_or(1.0);
        
        if rng.gen_bool((self.config.storm_chance * storm_mod * hour_to_check).min(1.0)) {
            self.spawn_storm(now_ms, rng);
        }

        // Tornado check - only in warm seasons with high temps
        let can_tornado = climate.as_ref().map(|c| {
            matches!(c.season, types::Season::Spring | types::Season::Summer) && c.temperature > 0.4
        }).unwrap_or(true);
        
        if can_tornado && rng.gen_bool((self.config.tornado_chance * hour_to_check).min(1.0)) {
            self.spawn_tornado(now_ms, rng);
        }

        // Rain check - more common in spring/fall
        let rain_mod = climate.as_ref().map(|c| match c.season {
            types::Season::Spring => 1.8,
            types::Season::Summer => 1.2,
            types::Season::Fall => 1.5,
            types::Season::Winter => 0.7,  // Less rain, more snow (TODO)
        }).unwrap_or(1.0);
        
        if rng.gen_bool((self.config.rain_chance * rain_mod * hour_to_check).min(1.0)) {
            self.spawn_rain(now_ms, rng);
        }
    }

    fn spawn_storm(&mut self, now_ms: i64, rng: &mut impl Rng) {
        let x = rng.gen_range(self.config.bounds_min[0]..self.config.bounds_max[0]);
        let z = rng.gen_range(self.config.bounds_min[2]..self.config.bounds_max[2]);
        
        let event = WeatherEvent {
            id: format!("storm_{}", now_ms),
            event_type: WeatherEventType::Storm {
                intensity: rng.gen_range(0.5..1.0),
                radius: rng.gen_range(50.0..150.0),
            },
            position: [x, 50.0, z],
            start_time_ms: now_ms,
            duration_ms: rng.gen_range(60_000..180_000), // 1-3 minutes
            is_active: true,
        };

        info!("Spawned storm at [{:.1}, {:.1}]", x, z);
        self.snapshot.active_events.push(event);
    }

    fn spawn_tornado(&mut self, now_ms: i64, rng: &mut impl Rng) {
        let x = rng.gen_range(self.config.bounds_min[0]..self.config.bounds_max[0]);
        let z = rng.gen_range(self.config.bounds_min[2]..self.config.bounds_max[2]);
        let direction = rng.gen_range(0.0..std::f64::consts::TAU);
        
        let event = WeatherEvent {
            id: format!("tornado_{}", now_ms),
            event_type: WeatherEventType::Tornado {
                intensity: rng.gen_range(0.7..1.0),
                radius: rng.gen_range(20.0..50.0),
                direction,
                speed: rng.gen_range(5.0..15.0), // m/s
            },
            position: [x, 0.0, z],
            start_time_ms: now_ms,
            duration_ms: rng.gen_range(30_000..120_000), // 30s-2min
            is_active: true,
        };

        warn!("Spawned TORNADO at [{:.1}, {:.1}] moving at {:.1} m/s", x, z, 
            if let WeatherEventType::Tornado { speed, .. } = event.event_type { speed } else { 0.0 });
        self.snapshot.active_events.push(event);
    }

    fn spawn_rain(&mut self, now_ms: i64, rng: &mut impl Rng) {
        let event = WeatherEvent {
            id: format!("rain_{}", now_ms),
            event_type: WeatherEventType::Rain {
                intensity: rng.gen_range(0.2..0.9),
                duration_seconds: rng.gen_range(120.0..600.0), // 2-10 minutes
            },
            position: [0.0, 0.0, 0.0], // Zone-wide
            start_time_ms: now_ms,
            duration_ms: rng.gen_range(120_000..600_000),
            is_active: true,
        };

        info!("Spawned rain event (intensity: {:.2})", 
            if let WeatherEventType::Rain { intensity, .. } = event.event_type { intensity } else { 0.0 });
        self.snapshot.active_events.push(event);
    }

    fn update_base_conditions(&mut self, rng: &mut impl Rng) {
        // Gradually drift wind speed and direction
        self.snapshot.base_wind_speed += rng.gen_range(-0.5..0.5);
        self.snapshot.base_wind_speed = self.snapshot.base_wind_speed.clamp(0.0, 15.0);

        self.snapshot.base_wind_direction += rng.gen_range(-0.1..0.1);

        // Update cloud cover based on rain
        let has_rain = self.snapshot.active_events.iter().any(|e| matches!(e.event_type, WeatherEventType::Rain { .. }));
        if has_rain {
            self.snapshot.cloud_cover = (self.snapshot.cloud_cover + 0.05).min(1.0);
            self.snapshot.precipitation = 0.8;
        } else {
            self.snapshot.cloud_cover += rng.gen_range(-0.02..0.02);
            self.snapshot.cloud_cover = self.snapshot.cloud_cover.clamp(0.1, 0.9);
            self.snapshot.precipitation *= 0.95; // Decay
        }

        // Visibility affected by fog/rain
        if has_rain {
            self.snapshot.visibility = 200.0;
        } else {
            self.snapshot.visibility = 1000.0;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("weather_sim=debug,info")
        .init();

    info!("Weather Sim v0.1.0 starting...");

    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    // Configure zones
    let configs = vec![
        WeatherConfig::new("test_zone".to_string()),
    ];

    let mut engine = WeatherEngine::new(&redis_url, configs).await?;  // 0.1Hz = 10 second ticks
    info!("Weather engine initialized for {} zones", engine.zones.len());

    engine.run().await
}
