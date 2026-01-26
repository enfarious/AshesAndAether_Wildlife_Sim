use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use futures_util::StreamExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum WeatherEventType {
    Storm { intensity: f64, radius: f64 },
    Tornado { intensity: f64, radius: f64, direction: f64, speed: f64 },
    Rain { intensity: f64, duration_seconds: f64 },
    Wind { speed: f64, direction: f64, gust_factor: f64 },
    Fog { density: f64, visibility: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherEvent {
    pub id: String,
    pub event_type: WeatherEventType,
    pub position: [f64; 3],
    pub start_time_ms: i64,
    pub duration_ms: i64,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherSnapshot {
    pub zone_id: String,
    pub timestamp_ms: i64,
    pub active_events: Vec<WeatherEvent>,
    pub base_wind_speed: f64,
    pub base_wind_direction: f64,
    pub precipitation: f64,
    pub cloud_cover: f64,
    pub visibility: f64,
}

pub struct WeatherCache {
    zone_id: String,
    cache: Arc<RwLock<Option<WeatherSnapshot>>>,
    _handle: Option<JoinHandle<()>>,
}

impl WeatherCache {
    pub fn new(zone_id: String) -> Self {
        Self {
            zone_id,
            cache: Arc::new(RwLock::new(None)),
            _handle: None,
        }
    }

    pub async fn start_subscription(&mut self, redis_url: &str) -> anyhow::Result<()> {
        let client = redis::Client::open(redis_url)?;
        let mut conn = client.get_multiplexed_tokio_connection().await?;
        #[allow(deprecated)]
        let pubsub_conn = client.get_async_connection().await?;
        let mut pubsub = pubsub_conn.into_pubsub();

        let channel = format!("weather:zone:{}", self.zone_id);
        pubsub.subscribe(channel).await?;

        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            let mut stream = pubsub.on_message();
            while let Some(msg) = stream.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(err) => {
                        eprintln!("weather_subscriber: failed to read payload: {}", err);
                        continue;
                    }
                };

                match serde_json::from_str::<WeatherSnapshot>(&payload) {
                    Ok(snapshot) => {
                        let mut guard = cache.write().await;
                        *guard = Some(snapshot);
                    }
                    Err(err) => eprintln!("weather_subscriber: failed to parse snapshot: {}", err),
                }
            }
        });

        self._handle = Some(handle);
        // Prime the cache with a blocking fetch if available (optional)
        if let Ok(Some(json)) = conn.get::<_, Option<String>>(format!("weather:snapshot:{}", self.zone_id)).await {
            if let Ok(snapshot) = serde_json::from_str::<WeatherSnapshot>(&json) {
                let mut guard = self.cache.write().await;
                *guard = Some(snapshot);
            }
        }

        Ok(())
    }

    pub async fn get(&self) -> Option<WeatherSnapshot> {
        self.cache.read().await.clone()
    }
}
