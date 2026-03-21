//! Spot Price Feed
//!
//! External price sources for fair value calculations.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use serde_json::Value;

const RTDS_WS_URL: &str = "wss://ws-live-data.polymarket.com";

/// Trait for external spot price sources
pub trait SpotFeed: Send + Sync {
    /// Get the current spot price
    fn get_price(&self) -> Option<f64>;

    /// Check if the feed is healthy (not stale)
    fn is_healthy(&self) -> bool;

    /// Update the feed (blocking version for sync contexts)
    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>>;

    /// Get feed name for debugging
    fn name(&self) -> &str {
        "unknown"
    }
}

/// Chainlink Data Feed configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChainlinkConfig {
    /// Asset symbol (e.g., "btc/usd")
    pub asset: String,
    /// API endpoint override
    pub endpoint: Option<String>,
    /// Staleness threshold in seconds
    pub staleness_threshold: u64,
}

impl Default for ChainlinkConfig {
    fn default() -> Self {
        Self {
            asset: "btc/usd".to_string(),
            endpoint: None,
            staleness_threshold: 60,
        }
    }
}

/// Chainlink HTTP price feed (Legacy/Polling)
pub struct ChainlinkFeed {
    endpoint: String,
    last_price: Option<f64>,
    last_update: u64,
    staleness_threshold: u64,
}

impl ChainlinkFeed {
    pub fn new(asset: &str) -> Self {
        Self {
            endpoint: format!("https://feeds.chain.link/{}", asset.to_lowercase()),
            last_price: None,
            last_update: 0,
            staleness_threshold: 60,
        }
    }

    pub fn from_config(config: ChainlinkConfig) -> Self {
        let endpoint = config.endpoint.unwrap_or_else(|| {
            format!("https://feeds.chain.link/{}", config.asset.to_lowercase())
        });

        Self {
            endpoint,
            last_price: None,
            last_update: 0,
            staleness_threshold: config.staleness_threshold,
        }
    }

    pub fn set_price(&mut self, price: f64) {
        self.last_price = Some(price);
        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    pub async fn fetch_async(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let response = client.get(&self.endpoint).send().await?;
        let data: ChainlinkResponse = response.json().await?;

        let price = self.parse_price(&data)?;

        self.last_price = Some(price);
        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        Ok(())
    }

    fn parse_price(&self, data: &ChainlinkResponse) -> Result<f64, Box<dyn std::error::Error>> {
        if let Some(price) = data.price {
            return Ok(price);
        }
        if let Some(value) = data.get("value").and_then(|v| v.as_str()) {
            if let Ok(price) = value.parse::<f64>() {
                return Ok(price);
            }
        }
        Err("Could not parse price from Chainlink response".into())
    }
}

impl SpotFeed for ChainlinkFeed {
    fn get_price(&self) -> Option<f64> {
        self.last_price
    }

    fn is_healthy(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now.saturating_sub(self.last_update) < self.staleness_threshold
    }

    fn name(&self) -> &str {
        "chainlink_polling"
    }

    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Err("Chainlink feed requires async update".into())
    }
}

/// Polymarket RTDS WebSocket Feed for Chainlink Prices
pub struct PolymarketRtdsFeed {
    asset: String,
    last_price: Option<f64>,
    last_update: u64,
    staleness_threshold: u64,
}

impl PolymarketRtdsFeed {
    pub fn new(asset: &str) -> Self {
        Self {
            asset: asset.to_lowercase(),
            last_price: None,
            last_update: 0,
            staleness_threshold: 30, // Faster staleness check for WS
        }
    }

    pub fn set_price(&mut self, price: f64) {
        self.last_price = Some(price);
        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }
}

impl SpotFeed for PolymarketRtdsFeed {
    fn get_price(&self) -> Option<f64> {
        self.last_price
    }

    fn is_healthy(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now.saturating_sub(self.last_update) < self.staleness_threshold
    }

    fn name(&self) -> &str {
        "polymarket_rtds"
    }

    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}

/// Wrapper for a shared spot feed (e.g., Arc<Mutex<T>>)
pub struct SharedSpotFeed<T: SpotFeed> {
    inner: Arc<TokioMutex<T>>,
    name: String,
}

impl<T: SpotFeed> SharedSpotFeed<T> {
    pub fn new(inner: Arc<TokioMutex<T>>, name: &str) -> Self {
        Self {
            inner,
            name: name.to_string(),
        }
    }
}

impl<T: SpotFeed> SpotFeed for SharedSpotFeed<T> {
    fn get_price(&self) -> Option<f64> {
        // Since SpotFeed::get_price is sync, we use try_lock
        if let Ok(guard) = self.inner.try_lock() {
            guard.get_price()
        } else {
            None
        }
    }

    fn is_healthy(&self) -> bool {
        if let Ok(guard) = self.inner.try_lock() {
            guard.is_healthy()
        } else {
            false
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}

/// Start a background poller for Polymarket RTDS WebSocket
pub fn start_rtds_poller(
    feed: Arc<TokioMutex<PolymarketRtdsFeed>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let asset = {
            let f = feed.lock().await;
            f.asset.clone()
        };

        loop {
            println!("[RTDS] Connecting to {}...", RTDS_WS_URL);
            let stream = connect_async(RTDS_WS_URL).await;
            let Ok((ws_stream, _)) = stream else {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            };

            let (mut write, mut read) = ws_stream.split();
            
            // RTDS uses slash-separated symbols like "btc/usd"
            let subscribe = serde_json::json!({
                "action": "subscribe",
                "subscriptions": [
                    {
                        "topic": "crypto_prices_chainlink",
                        "type": "*",
                        "filters": format!("{{\"symbol\":\"{}\"}}", asset)
                    }
                ]
            });

            if write.send(Message::Text(subscribe.to_string().into())).await.is_err() {
                continue;
            }

            // Heartbeat: send PING every 5 seconds
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));

            loop {
                tokio::select! {
                    _ = heartbeat.tick() => {
                        if write.send(Message::Text("PING".into())).await.is_err() {
                            break;
                        }
                    }
                    message = read.next() => {
                        let Some(Ok(message)) = message else {
                            break;
                        };

                        let payload = match message {
                            Message::Text(text) => text.to_string(),
                            Message::Binary(bin) => String::from_utf8_lossy(&bin).to_string(),
                            _ => continue,
                        };

                        if payload == "PONG" {
                            continue;
                        }

                        let Ok(value): serde_json::Result<Value> = serde_json::from_str(&payload) else {
                            continue;
                        };

                        // Expected format:
                        // {
                        //   "topic": "crypto_prices_chainlink",
                        //   "payload": { "symbol": "btc/usd", "value": 67234.50, ... }
                        // }
                        if let Some(payload_obj) = value.get("payload") {
                            if let Some(price_val) = payload_obj.get("value").and_then(|v| v.as_f64()) {
                                let mut f = feed.lock().await;
                                f.set_price(price_val);
                            }
                        }
                    }
                }
            }

            println!("[RTDS] Connection lost, reconnecting in 5s...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    })
}

#[derive(Debug, Deserialize)]
pub struct ChainlinkResponse {
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
    pub price: Option<f64>,
}

impl ChainlinkResponse {
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.extra.get(key)
    }
}

pub fn start_chainlink_poller(
    feed: Arc<TokioMutex<ChainlinkFeed>>,
    interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            let mut feed = feed.lock().await;
            if let Err(e) = feed.fetch_async().await {
                eprintln!("[Chainlink] Fetch error: {}", e);
            }
        }
    })
}

/// Derived spot price from Polymarket orderbook
#[derive(Debug, Clone, Default)]
pub struct DerivedSpotFeed {
    last_price: Option<f64>,
    last_update: u64,
    staleness_threshold: u64,
}

impl DerivedSpotFeed {
    pub fn new(staleness_threshold: u64) -> Self {
        Self {
            last_price: None,
            last_update: 0,
            staleness_threshold,
        }
    }

    pub fn update_from_market(
        &mut self,
        yes_mid: f64,
        _no_mid: f64,
        _time_remaining: i64,
    ) -> Option<f64> {
        self.last_price = Some(yes_mid);
        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.last_price
    }
}

impl SpotFeed for DerivedSpotFeed {
    fn get_price(&self) -> Option<f64> {
        self.last_price
    }

    fn is_healthy(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now.saturating_sub(self.last_update) < self.staleness_threshold
    }

    fn name(&self) -> &str {
        "derived"
    }

    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}

/// Composite spot feed
pub struct CompositeSpotFeed {
    primary: Option<Box<dyn SpotFeed>>,
    secondary: Option<DerivedSpotFeed>,
    fallback: Option<f64>,
    active_source: String,
}

impl CompositeSpotFeed {
    pub fn new() -> Self {
        Self {
            primary: None,
            secondary: None,
            fallback: None,
            active_source: "none".to_string(),
        }
    }

    pub fn with_primary(mut self, feed: Box<dyn SpotFeed>) -> Self {
        self.primary = Some(feed);
        self
    }

    pub fn with_secondary(mut self, feed: DerivedSpotFeed) -> Self {
        self.secondary = Some(feed);
        self
    }

    pub fn with_fallback(mut self, price: f64) -> Self {
        self.fallback = Some(price);
        self
    }

    pub fn update_derived(&mut self, yes_mid: f64, no_mid: f64) {
        if let Some(ref mut derived) = self.secondary {
            derived.update_from_market(yes_mid, no_mid, 0);
        }
    }

    pub fn active_source(&self) -> &str {
        if let Some(ref primary) = self.primary {
            if primary.is_healthy() {
                return primary.name();
            }
        }
        if let Some(ref secondary) = self.secondary {
            if secondary.is_healthy() {
                return secondary.name();
            }
        }
        if self.fallback.is_some() {
            return "fallback";
        }
        "none"
    }
}

impl SpotFeed for CompositeSpotFeed {
    fn get_price(&self) -> Option<f64> {
        if let Some(ref primary) = self.primary {
            if primary.is_healthy() {
                if let Some(price) = primary.get_price() {
                    return Some(price);
                }
            }
        }

        if let Some(ref secondary) = self.secondary {
            if secondary.is_healthy() {
                if let Some(price) = secondary.get_price() {
                    return Some(price);
                }
            }
        }

        self.fallback
    }

    fn is_healthy(&self) -> bool {
        self.primary.as_ref().map(|p| p.is_healthy()).unwrap_or(false)
            || self.secondary.as_ref().map(|s| s.is_healthy()).unwrap_or(false)
            || self.fallback.is_some()
    }

    fn name(&self) -> &str {
        "composite"
    }

    fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut primary) = self.primary {
            let _ = primary.update();
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_spot_feed_initially_empty() {
        let feed = DerivedSpotFeed::new(60);
        assert!(feed.get_price().is_none());
        assert!(!feed.is_healthy());
    }

    #[test]
    fn derived_spot_feed_updates_from_market() {
        let mut feed = DerivedSpotFeed::new(60);
        let price = feed.update_from_market(0.55, 0.45, 300);

        assert!(price.is_some());
        assert_eq!(price.unwrap(), 0.55);
        assert!(feed.is_healthy());
    }

    #[test]
    fn chainlink_feed_default_config() {
        let config = ChainlinkConfig::default();
        assert_eq!(config.asset, "btc_usd");
        assert_eq!(config.staleness_threshold, 60);
    }

    #[test]
    fn chainlink_feed_from_config() {
        let config = ChainlinkConfig {
            asset: "eth_usd".to_string(),
            endpoint: None, // Using default endpoint
            staleness_threshold: 120,
        };

        let feed = ChainlinkFeed::from_config(config);
        assert_eq!(feed.staleness_threshold, 120);
        assert!(feed.is_healthy());
    }

    #[test]
    fn chainlink_feed_set_price() {
        let mut feed = ChainlinkFeed::new("btc_usd");
        assert!(!feed.is_healthy());

        feed.set_price(50000.0);
        assert!(feed.is_healthy());
        assert_eq!(feed.get_price(), Some(50000.0));
    }

    #[test]
    fn composite_feed_uses_primary_first() {
        let mut chainlink = ChainlinkFeed::new("btc_usd");
        chainlink.set_price(50000.0);

        let composite = CompositeSpotFeed::new()
            .with_primary(Box::new(chainlink))
            .with_fallback(48000.0);

        // Should use primary (Chainlink)
        assert_eq!(composite.get_price(), Some(50000.0));
    }

    #[test]
    fn composite_feed_falls_back_to_secondary() {
        let chainlink = ChainlinkFeed::new("btc_usd");
        // Don't set price - unhealthy

        let mut derived = DerivedSpotFeed::new(60);
        derived.update_from_market(0.55, 0.45, 300);

        let composite = CompositeSpotFeed::new()
            .with_primary(Box::new(chainlink))
            .with_secondary(derived)
            .with_fallback(48000.0);

        // Should fall back to derived
        assert_eq!(composite.get_price(), Some(0.55));
    }

    #[test]
    fn composite_feed_uses_fallback() {
        let chainlink = ChainlinkFeed::new("btc_usd");
        let derived = DerivedSpotFeed::new(60);
        // Neither is updated

        let composite = CompositeSpotFeed::new()
            .with_primary(Box::new(chainlink))
            .with_secondary(derived)
            .with_fallback(48000.0);

        // Should use fallback
        assert_eq!(composite.get_price(), Some(48000.0));
    }

    #[test]
    fn composite_feed_health_check() {
        let mut chainlink = ChainlinkFeed::new("btc_usd");
        chainlink.set_price(50000.0);

        let composite = CompositeSpotFeed::new()
            .with_primary(Box::new(chainlink));

        assert!(composite.is_healthy());
    }

    #[test]
    fn composite_feed_updates_derived() {
        let chainlink = ChainlinkFeed::new("btc_usd");
        let mut derived = DerivedSpotFeed::new(60);

        let mut composite = CompositeSpotFeed::new()
            .with_primary(Box::new(chainlink))
            .with_secondary(derived);

        composite.update_derived(0.60, 0.40);

        // Now derived should be healthy and provide price
        assert_eq!(composite.get_price(), Some(0.60));
    }
}
