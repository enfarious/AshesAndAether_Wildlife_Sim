//! Climate subscriber - receives climate updates from climate_sim via Redis
//!
//! Caches climate state locally for fast access by simulation.

use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

/// Cached climate state with local storage
pub struct ClimateCache {
    cached: Arc<RwLock<Option<ClimateSnapshot>>>,
    zone_id: String,
}

impl ClimateCache {
    pub fn new(zone_id: String) -> Self {
        Self {
            cached: Arc::new(RwLock::new(None)),
            zone_id,
        }
    }

    /// Start subscribing to climate updates from Redis
    pub async fn start_subscription(&self, redis_url: &str) -> Result<()> {
        let client = redis::Client::open(redis_url)?;
        let mut pubsub = client.get_async_pubsub().await?;

        let channel = format!("climate:zone:{}", self.zone_id);
        pubsub.subscribe(&channel).await?;
        info!("Subscribed to climate updates: {}", channel);

        let cached = Arc::clone(&self.cached);

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

    /// Get current climate state (blocks if no data yet)
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
                error!("Timeout waiting for climate data for zone {}", self.zone_id);
                return None;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
}
