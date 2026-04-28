use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use redis::AsyncCommands;
use serde::{Deserialize, Deserializer, Serialize};
use futures_util::StreamExt;

/// Accept both JSON integers and floats, truncating to i64.
/// Needed because the TypeScript game server serializes numbers as floats.
fn deserialize_i64_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if let Some(f) = n.as_f64() {
                Ok(f as i64)
            } else {
                Err(serde::de::Error::custom("expected numeric value"))
            }
        }
        _ => Err(serde::de::Error::custom("expected number")),
    }
}

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
    #[serde(deserialize_with = "deserialize_i64_lenient")]
    pub start_time_ms: i64,
    #[serde(deserialize_with = "deserialize_i64_lenient")]
    pub duration_ms: i64,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherSnapshot {
    pub zone_id: String,
    #[serde(deserialize_with = "deserialize_i64_lenient")]
    pub timestamp_ms: i64,
    pub active_events: Vec<WeatherEvent>,
    pub base_wind_speed: f64,
    pub base_wind_direction: f64,
    pub precipitation: f64,
    pub cloud_cover: f64,
    pub visibility: f64,
}

/// How long (ms) before we consider external weather data stale.
/// The weather_sim publishes at 0.1 Hz (every 10s), so 30s means ~3 missed updates.
const WEATHER_STALE_THRESHOLD_MS: u64 = 30_000;

pub struct WeatherCache {
    zone_id: String,
    cache: Arc<RwLock<Option<WeatherSnapshot>>>,
    /// Wall-clock millis when the last external snapshot was received.
    last_received_ms: Arc<AtomicU64>,
    _handle: Option<JoinHandle<()>>,
}

impl WeatherCache {
    pub fn new(zone_id: String) -> Self {
        Self {
            zone_id,
            cache: Arc::new(RwLock::new(None)),
            last_received_ms: Arc::new(AtomicU64::new(0)),
            _handle: None,
        }
    }

    /// Returns true if the external weather_sim has published fresh data recently.
    pub fn is_external_active(&self) -> bool {
        let last = self.last_received_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        now.saturating_sub(last) < WEATHER_STALE_THRESHOLD_MS
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
        let last_received = self.last_received_ms.clone();
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
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        last_received.store(now_ms, Ordering::Relaxed);
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
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                self.last_received_ms.store(now_ms, Ordering::Relaxed);
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
