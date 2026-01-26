use serde::{Deserialize, Serialize};

/// Climate snapshot (from climate_sim)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateSnapshot {
    pub zone_id: String,
    pub timestamp_ms: i64,
    pub day_of_year: u16,
    pub time_of_day: f64,
    pub year: u32,
    pub season: Season,
    pub temperature: f64,
    pub day_length: f64,
    pub is_night: bool,
    pub growth_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Season {
    Spring,
    Summer,
    Fall,
    Winter,
}

/// Weather event types that can occur in zones
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum WeatherEventType {
    Storm {
        intensity: f64,      // 0.0-1.0
        radius: f64,         // meters
    },
    Tornado {
        intensity: f64,      // 0.0-1.0
        radius: f64,         // meters
        direction: f64,      // radians
        speed: f64,          // m/s
    },
    Rain {
        intensity: f64,      // 0.0-1.0 (drizzle to downpour)
        duration_seconds: f64,
    },
    Wind {
        speed: f64,          // m/s
        direction: f64,      // radians
        gust_factor: f64,    // 1.0-2.0 multiplier for gusts
    },
    Fog {
        density: f64,        // 0.0-1.0
        visibility: f64,     // meters
    },
}

/// Active weather event in a zone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherEvent {
    pub id: String,
    pub event_type: WeatherEventType,
    pub position: [f64; 3],    // [x, y, z] center point
    pub start_time_ms: i64,
    pub duration_ms: i64,
    pub is_active: bool,
}

/// Weather snapshot published to Redis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherSnapshot {
    pub zone_id: String,
    pub timestamp_ms: i64,
    pub active_events: Vec<WeatherEvent>,
    pub base_wind_speed: f64,      // m/s
    pub base_wind_direction: f64,  // radians
    pub precipitation: f64,         // 0.0-1.0
    pub cloud_cover: f64,           // 0.0-1.0
    pub visibility: f64,            // meters
}

impl WeatherSnapshot {
    pub fn calm(zone_id: String, timestamp_ms: i64) -> Self {
        Self {
            zone_id,
            timestamp_ms,
            active_events: Vec::new(),
            base_wind_speed: 2.0,
            base_wind_direction: 0.0,
            precipitation: 0.0,
            cloud_cover: 0.3,
            visibility: 1000.0,
        }
    }
}

/// Weather configuration for a zone
#[derive(Debug, Clone)]
pub struct WeatherConfig {
    pub zone_id: String,
    pub storm_chance: f64,        // per hour probability
    pub tornado_chance: f64,      // per hour probability
    pub rain_chance: f64,         // per hour probability
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}

impl WeatherConfig {
    pub fn new(zone_id: String) -> Self {
        Self {
            zone_id,
            storm_chance: 0.05,      // 5% per hour
            tornado_chance: 0.01,    // 1% per hour
            rain_chance: 0.15,       // 15% per hour
            bounds_min: [-500.0, 0.0, -500.0],
            bounds_max: [500.0, 100.0, 500.0],
        }
    }
}
