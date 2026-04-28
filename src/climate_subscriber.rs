//! Climate subscriber - receives climate updates from climate_sim via Redis
//!
//! Caches climate state locally for fast access by simulation.
//! Tracks freshness so the simulation can fall back to its internal
//! climate when the external climate_sim is not running.

use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Climate snapshot received from climate_sim
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateSnapshot {
    pub zone_id: String,
    pub day_of_year: u16,
    pub time_of_day: f64,
    pub year: u32,
    pub season: Season,
    pub temperature: f64,
    pub day_length: f64,
    pub is_night: bool,
    pub growth_rate: f64,
    pub timestamp: u64,
}

/// Season enum matching climate_sim
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Fall,
    Winter,
}

/// How long (ms) before we consider external climate data stale.
/// The climate_sim publishes at 1 Hz, so 5 seconds means ~5 missed updates.
const EXTERNAL_STALE_THRESHOLD_MS: u64 = 5000;

/// Cached climate state with local storage and freshness tracking.
pub struct ClimateCache {
    cached: Arc<RwLock<Option<ClimateSnapshot>>>,
    /// Wall-clock millis when the last external snapshot was received.
    last_received_ms: Arc<AtomicU64>,
    zone_id: String,
}

impl ClimateCache {
    pub fn new(zone_id: String) -> Self {
        Self {
            cached: Arc::new(RwLock::new(None)),
            last_received_ms: Arc::new(AtomicU64::new(0)),
            zone_id,
        }
    }

    /// Returns true if the external climate_sim has published fresh data recently.
    pub fn is_external_active(&self) -> bool {
        let last = self.last_received_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        now.saturating_sub(last) < EXTERNAL_STALE_THRESHOLD_MS
    }

    /// Start subscribing to climate updates from Redis
    pub async fn start_subscription(&self, redis_url: &str) -> Result<()> {
        let client = redis::Client::open(redis_url)?;
        let mut pubsub = client.get_async_pubsub().await?;

        let channel = format!("climate:zone:{}", self.zone_id);
        pubsub.subscribe(&channel).await?;
        info!("Subscribed to climate updates: {}", channel);

        let cached = Arc::clone(&self.cached);
        let last_received = Arc::clone(&self.last_received_ms);

        // Spawn background task to receive updates
        tokio::spawn(async move {
            loop {
                let msg = pubsub.on_message().next().await;
                if let Some(msg) = msg {
                    let payload: String = match msg.get_payload() {
                        Ok(p) => p,
                        Err(e) => {
                            error!("Failed to get climate payload: {}", e);
                            continue;
                        }
                    };

                    match serde_json::from_str::<ClimateSnapshot>(&payload) {
                        Ok(snapshot) => {
                            debug!(
                                "Climate update: Day {} {:?} Temp:{:.2}",
                                snapshot.day_of_year, snapshot.season, snapshot.temperature
                            );
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64;
                            last_received.store(now_ms, Ordering::Relaxed);
                            *cached.write().await = Some(snapshot);
                        }
                        Err(e) => {
                            error!("Failed to parse climate snapshot: {}", e);
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Get current climate state (non-blocking)
    pub async fn get(&self) -> Option<ClimateSnapshot> {
        self.cached.read().await.clone()
    }

    /// Get current climate state, or wait up to timeout_ms for first update
    pub async fn get_or_wait(&self, timeout_ms: u64) -> Option<ClimateSnapshot> {
        let start = std::time::Instant::now();
        loop {
            if let Some(snapshot) = self.get().await {
                return Some(snapshot);
            }

            if start.elapsed().as_millis() > timeout_ms as u128 {
                info!("No external climate data after {}ms for zone {} — will use internal fallback", timeout_ms, self.zone_id);
                return None;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
}
