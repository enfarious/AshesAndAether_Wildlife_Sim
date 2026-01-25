//! Redis communication bridge
//!
//! Handles pub/sub messaging between the wildlife sim and game server.

use crate::types::*;
use anyhow::Result;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Redis channels
pub mod channels {
    /// Wildlife sim publishes events here
    pub const WILDLIFE_EVENTS: &str = "wildlife:events";

    /// Game server publishes player updates here
    pub const GAME_PLAYERS: &str = "wildlife:players";

    /// Game server publishes zone info here
    pub const GAME_ZONES: &str = "wildlife:zones";

    /// Game server publishes combat events here
    pub const GAME_COMBAT: &str = "wildlife:combat";

    /// State sync requests/responses
    pub const STATE_SYNC: &str = "wildlife:sync";
}

/// Bridge between simulation and Redis
pub struct RedisBridge {
    connection: MultiplexedConnection,
    pubsub_rx: mpsc::Receiver<GameServerMessage>,
}

impl RedisBridge {
    /// Connect to Redis and set up pub/sub
    pub async fn connect(redis_url: &str) -> Result<Self> {
        info!("Connecting to Redis at {}", redis_url);

        let client = redis::Client::open(redis_url)?;
        let connection = client.get_multiplexed_async_connection().await?;

        // Set up pub/sub receiver
        let (tx, rx) = mpsc::channel(1000);

        // Spawn subscription handler
        let mut pubsub_conn = client.get_async_pubsub().await?;
        pubsub_conn.subscribe(channels::GAME_PLAYERS).await?;
        pubsub_conn.subscribe(channels::GAME_ZONES).await?;
        pubsub_conn.subscribe(channels::GAME_COMBAT).await?;
        pubsub_conn.subscribe(channels::STATE_SYNC).await?;

        tokio::spawn(async move {
            Self::subscription_handler(pubsub_conn, tx).await;
        });

        info!("Redis connection established");

        Ok(Self {
            connection,
            pubsub_rx: rx,
        })
    }

    async fn subscription_handler(
        mut pubsub: redis::aio::PubSub,
        tx: mpsc::Sender<GameServerMessage>,
    ) {
        use futures_util::StreamExt;

        let mut stream = pubsub.on_message();

        while let Some(msg) = stream.next().await {
            let payload: String = match msg.get_payload() {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to get message payload: {}", e);
                    continue;
                }
            };

            let channel: String = msg.get_channel_name().to_string();

            match serde_json::from_str::<GameServerMessage>(&payload) {
                Ok(message) => {
                    debug!("Received message on {}: {:?}", channel, message);
                    if tx.send(message).await.is_err() {
                        error!("Failed to send message to handler");
                        break;
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to parse message on {}: {} - payload: {}",
                        channel, e, payload
                    );
                }
            }
        }
    }

    /// Receive next message from game server (non-blocking)
    pub fn try_recv(&mut self) -> Option<GameServerMessage> {
        self.pubsub_rx.try_recv().ok()
    }

    /// Publish wildlife events to game server
    pub async fn publish_events(&mut self, events: Vec<WildlifeEvent>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        for event in events {
            let payload = serde_json::to_string(&event)?;
            self.connection
                .publish::<_, _, ()>(channels::WILDLIFE_EVENTS, payload)
                .await?;
        }

        Ok(())
    }

    /// Publish full state for a zone (for sync)
    pub async fn publish_state(
        &mut self,
        zone_id: &str,
        wildlife: &[WildlifeEntity],
        plants: &[PlantEntity],
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct StatePayload<'a> {
            zone_id: &'a str,
            wildlife: &'a [WildlifeEntity],
            plants: &'a [PlantEntity],
            timestamp: i64,
        }

        let payload = serde_json::to_string(&StatePayload {
            zone_id,
            wildlife,
            plants,
            timestamp: chrono::Utc::now().timestamp_millis(),
        })?;

        self.connection
            .publish::<_, _, ()>(channels::STATE_SYNC, payload)
            .await?;

        Ok(())
    }
}

/// Simplified bridge for testing without Redis
pub struct MockBridge {
    pub outgoing_events: Vec<WildlifeEvent>,
    pub incoming_messages: Vec<GameServerMessage>,
}

impl MockBridge {
    pub fn new() -> Self {
        Self {
            outgoing_events: Vec::new(),
            incoming_messages: Vec::new(),
        }
    }

    pub fn try_recv(&mut self) -> Option<GameServerMessage> {
        if self.incoming_messages.is_empty() {
            None
        } else {
            Some(self.incoming_messages.remove(0))
        }
    }

    pub fn publish_events(&mut self, events: Vec<WildlifeEvent>) {
        self.outgoing_events.extend(events);
    }
}
