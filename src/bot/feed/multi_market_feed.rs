//! Multi-Market Feed for Temporal Arbitrage
//!
//! This module provides a WebSocket-based feed for streaming orderbook
//! data from multiple markets simultaneously.
//!
//! This is essential for the temporal arbitrage strategy which needs
//! to monitor multiple nested timeframes at once.

use crate::bot::feed_base::DualBookState;
use crate::bot::feed_base::MarketSnapshot;
use crate::bot::logging::JsonlEventLogger;
use crate::bot::strategy::Direction;
use anyhow::Result;
use futures_util::{StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CLOB_MARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

/// Timeframe enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Timeframe {
    M5,
    M15,
    H1,
    H4,
}

/// Market subscription information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSubscription {
    /// Unique condition ID
    pub condition_id: String,
    /// YES token ID
    pub yes_token_id: String,
    /// NO token ID
    pub no_token_id: String,
    /// Market timeframe
    pub timeframe: Timeframe,
    /// Market start time (Unix timestamp)
    pub start_time: i64,
    /// Market end time (Unix timestamp)
    pub end_time: i64,
    /// Strike price
    pub strike_price: f64,
}

/// Market event from the feed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEvent {
    /// Condition ID
    pub condition_id: String,
    /// YES bid price
    pub yes_bid: f64,
    /// YES ask price
    pub yes_ask: f64,
    /// NO bid price
    pub no_bid: f64,
    /// NO ask price
    pub no_ask: f64,
    /// Timestamp
    pub ts: i64,
    /// YES midpoint (for convenience)
    pub yes_mid: f64,
    /// NO midpoint (for convenience)
    pub no_mid: f64,
    /// Top 5 bid depth
    pub yes_bid_depth: f64,
    /// Top 5 ask depth
    pub yes_ask_depth: f64,
}

impl MarketEvent {
    /// Create a new market event
    pub fn new(condition_id: String, yes_bid: f64, yes_ask: f64, no_bid: f64, no_ask: f64, ts: i64) -> Self {
        let yes_mid = (yes_bid + yes_ask) / 2.0;
        let no_mid = (no_bid + no_ask) / 2.0;

        Self {
            condition_id,
            yes_bid,
            yes_ask,
            no_bid,
            no_ask,
            ts,
            yes_mid,
            no_mid,
            yes_bid_depth: 0.0,
            yes_ask_depth: 0.0,
        }
    }

    /// Check if the event is valid
    pub fn is_valid(&self) -> bool {
        self.yes_bid > 0.0 && self.yes_ask > 0.0 &&
        self.no_bid > 0.0 && self.no_ask > 0.0 &&
        self.yes_ask >= self.yes_bid &&
        self.no_ask >= self.no_bid
    }

    /// Calculate the spread
    pub fn yes_spread(&self) -> f64 {
        self.yes_ask - self.yes_bid
    }

    /// Calculate the spread
    pub fn no_spread(&self) -> f64 {
        self.no_ask - self.no_bid
    }

    /// Check if the book is "broken" (yes_ask + no_ask != 1.0)
    pub fn is_broken_book(&self, tolerance: f64) -> bool {
        (self.yes_ask + self.no_ask - 1.0).abs() > tolerance
    }
}

/// Shared state for multi-market feed
#[derive(Debug)]
pub struct MultiMarketFeedState {
    /// Subscribed markets
    pub markets: HashMap<String, MarketSubscription>,
    /// Book state for each market
    pub book_states: HashMap<String, DualBookState>,
    /// Token ID to condition ID mapping
    pub token_to_market: HashMap<String, String>,
}

impl Default for MultiMarketFeedState {
    fn default() -> Self {
        Self {
            markets: HashMap::new(),
            book_states: HashMap::new(),
            token_to_market: HashMap::new(),
        }
    }
}

/// Multi-market WebSocket feed
pub struct MultiMarketWebsocketFeed {
    /// Market state
    state: Arc<Mutex<MultiMarketFeedState>>,
    /// Event receiver
    event_rx: mpsc::UnboundedReceiver<MarketEvent>,
    /// Join handle for the WebSocket task
    _join_handle: tokio::task::JoinHandle<()>,
}

impl MultiMarketWebsocketFeed {
    /// Connect to the WebSocket and subscribe to markets
    pub async fn connect(
        subscriptions: Vec<MarketSubscription>,
        logger: Option<JsonlEventLogger>,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(MultiMarketFeedState::default()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Collect all token IDs
        let all_tokens: Vec<String> = subscriptions.iter()
            .flat_map(|m| vec![m.yes_token_id.clone(), m.no_token_id.clone()])
            .collect();

        // Build market map and token mapping
        {
            let mut state_guard = state.lock().await;
            for sub in subscriptions {
                let condition_id = sub.condition_id.clone();
                state_guard.token_to_market.insert(sub.yes_token_id.clone(), condition_id.clone());
                state_guard.token_to_market.insert(sub.no_token_id.clone(), condition_id.clone());
                state_guard.markets.insert(sub.condition_id.clone(), sub.clone());
                state_guard.book_states.insert(sub.condition_id.clone(), DualBookState::default());
            }
        }

        // Extract token_to_market mapping for the websocket task
        let token_to_market = {
            let state_guard = state.lock().await;
            state_guard.token_to_market.clone()
        };

        // Spawn WebSocket task
        eprintln!("[multi-market-feed] Subscribing to {} tokens: {:?}", all_tokens.len(), all_tokens);
        let join_handle = tokio::spawn(async move {
            Self::websocket_task(all_tokens, token_to_market, event_tx, logger).await;
        });

        Ok(Self {
            state,
            event_rx,
            _join_handle: join_handle,
        })
    }

    /// WebSocket task
    async fn websocket_task(
        tokens: Vec<String>,
        token_to_market: HashMap<String, String>,
        event_tx: mpsc::UnboundedSender<MarketEvent>,
        logger: Option<JsonlEventLogger>,
    ) {
        let subscribe_msg = serde_json::json!({
            "type": "market",
            "assets_ids": tokens
        });

        eprintln!("[multi-market-feed] Token-to-market mapping has {} entries", token_to_market.len());

        loop {
            // Connect to WebSocket
            let stream = connect_async(CLOB_MARKET_WS_URL).await;
            let Ok((ws_stream, _)) = stream else {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            };

            let (mut write, mut read) = ws_stream.split();

            // Send subscription message
            if write.send(Message::Text(subscribe_msg.to_string().into())).await.is_err() {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            // Process messages
            let mut msg_count = 0;
            while let Some(message) = read.next().await {
                let Ok(message) = message else { break };

                let payload = match message {
                    Message::Text(text) => text,
                    Message::Ping(_) => continue,
                    Message::Pong(_) => continue,
                    Message::Close(_) => break,
                    _ => continue,
                };

                msg_count += 1;
                if msg_count <= 5 || msg_count % 100 == 0 {
                    eprintln!("[multi-market-feed] Received message #{}: {}", msg_count, payload.chars().take(200).collect::<String>());
                }

                // Parse JSON
                let Ok(value) = serde_json::from_str::<Value>(&payload) else {
                    if msg_count <= 5 {
                        eprintln!("[multi-market-feed] Failed to parse JSON");
                    }
                    continue;
                };

                // Process events
                if let Err(err) = Self::process_ws_message(&value, &token_to_market, &event_tx, &logger) {
                    eprintln!("[multi-market-feed] Error processing message: {}", err);
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    /// Process a WebSocket message
    fn process_ws_message(
        value: &Value,
        token_to_market: &HashMap<String, String>,
        event_tx: &mpsc::UnboundedSender<MarketEvent>,
        logger: &Option<JsonlEventLogger>,
    ) -> Result<()> {
        // Handle array of events
        if let Some(items) = value.as_array() {
            for item in items {
                Self::process_single_event(item, token_to_market, event_tx, logger)?;
            }
        } else {
            Self::process_single_event(value, token_to_market, event_tx, logger)?;
        }

        Ok(())
    }

    /// Process a single event
    fn process_single_event(
        value: &Value,
        token_to_market: &HashMap<String, String>,
        event_tx: &mpsc::UnboundedSender<MarketEvent>,
        logger: &Option<JsonlEventLogger>,
    ) -> Result<()> {
        // Try Polymarket's actual WebSocket format first
        // {"market":"0x...","price_changes":[{"asset_id":"...","price":"0.7",...}]}
        if let Some(events) = Self::parse_polymarket_ws_format(value, token_to_market) {
            for evt in events {
                eprintln!("[multi-market-feed] Sending event for condition_id={}, yes_mid={}",
                    evt.condition_id.chars().take(16).collect::<String>(), evt.yes_mid);
                if let Some(logger) = logger {
                    logger.log("multi_market_event", &evt);
                }
                if let Err(e) = event_tx.send(evt) {
                    eprintln!("[multi-market-feed] Failed to send event through channel: {}", e);
                }
            }
            return Ok(());
        }

        let event_type = value.get("event_type")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("type").and_then(|v| v.as_str()))
            .unwrap_or_default();

        let event = match event_type {
            "book" => Self::parse_book_event(value, token_to_market),
            "price_change" => Self::parse_price_change_event(value, token_to_market),
            _ => {
                // Try generic orderbook format
                Self::parse_orderbook_event(value, token_to_market)
            }
        };

        if let Some(evt) = event {
            if let Some(logger) = logger {
                logger.log("multi_market_event", &evt);
            }
            let _ = event_tx.send(evt);
        }

        Ok(())
    }

    /// Parse Polymarket's actual WebSocket message format
    /// {"market":"0x...","price_changes":[{"asset_id":"...","price":"0.7",...}]}
    fn parse_polymarket_ws_format(
        value: &Value,
        token_to_market: &HashMap<String, String>,
    ) -> Option<Vec<MarketEvent>> {
        // Check if this is the polymarket format (has "market" and "price_changes" fields)
        let market_id = value.get("market")?.as_str()?;

        // Try price_changes array format
        let price_changes = value.get("price_changes").and_then(|v| v.as_array());

        if let Some(changes) = price_changes {
            if !changes.is_empty() {
                eprintln!("[multi-market-feed] Trying to parse price_changes with {} changes, market={}, tokens_in_map={}",
                    changes.len(), market_id, token_to_market.len());
            }

            let mut events = Vec::new();
            let now = chrono::Utc::now().timestamp();

            for change in changes {
                let token_id = change.get("asset_id")?.as_str()?;

                // Debug: check if token_id is in our mapping
                let condition_id = match token_to_market.get(token_id) {
                    Some(id) => id,
                    None => {
                        eprintln!("[multi-market-feed] Token NOT in mapping: {}", token_id);
                        continue;
                    }
                };

                let price = change.get("price")?.as_str()?;
                let price_val: f64 = price.parse().ok()?;

                // Estimate spread
                let spread = 0.01;
                let yes_bid = (price_val - spread / 2.0).max(0.001);
                let yes_ask = (price_val + spread / 2.0).min(0.999);
                let no_bid = (1.0 - yes_ask).max(0.001);
                let no_ask = (1.0 - yes_bid).max(0.001);

                events.push(MarketEvent::new(
                    condition_id.clone(),
                    yes_bid,
                    yes_ask,
                    no_bid,
                    no_ask,
                    now,
                ));
            }

            if !events.is_empty() {
                eprintln!("[multi-market-feed] Parsed {} events from price_changes format", events.len());
                return Some(events);
            }
        }

        None
    }

    /// Parse a book event
    fn parse_book_event(
        value: &Value,
        token_to_market: &HashMap<String, String>,
    ) -> Option<MarketEvent> {
        let token_id = value.get("asset_id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("token_id").and_then(|v| v.as_str()))?;

        let condition_id = token_to_market.get(token_id)?;

        let yes_price = value.get("yes_price").and_then(|v| v.as_f64());
        let no_price = value.get("no_price").and_then(|v| v.as_f64());

        match (yes_price, no_price) {
            (Some(yp), Some(np)) => {
                let ts = value.get("timestamp")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() as f64 / 1000.0) as i64;

                // Estimate bid/ask from midpoint
                let spread = 0.01; // Default spread estimate
                Some(MarketEvent::new(
                    condition_id.clone(),
                    (yp - spread / 2.0).max(0.001),
                    (yp + spread / 2.0).min(0.999),
                    (np - spread / 2.0).max(0.001),
                    (np + spread / 2.0).min(0.999),
                    ts,
                ))
            }
            _ => None,
        }
    }

    /// Parse a price change event
    fn parse_price_change_event(
        value: &Value,
        token_to_market: &HashMap<String, String>,
    ) -> Option<MarketEvent> {
        let token_id = value.get("asset_id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("token_id").and_then(|v| v.as_str()))?;

        let condition_id = token_to_market.get(token_id)?;

        let best_bid = value.get("best_bid").and_then(|v| v.as_f64())?;
        let best_ask = value.get("best_ask").and_then(|v| v.as_f64())?;

        let ts = value.get("timestamp")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() as f64 / 1000.0) as i64;

        // Determine if this is YES or NO token
        // For now, assume NO = 1 - YES
        let yes_bid = best_bid.min(1.0);
        let yes_ask = best_ask.min(1.0);
        let no_bid = (1.0 - yes_ask).max(0.001);
        let no_ask = (1.0 - yes_bid).max(0.001);

        Some(MarketEvent::new(
            condition_id.clone(),
            yes_bid,
            yes_ask,
            no_bid,
            no_ask,
            ts,
        ))
    }

    /// Parse an orderbook snapshot event (different format)
    fn parse_orderbook_event(
        value: &Value,
        token_to_market: &HashMap<String, String>,
    ) -> Option<MarketEvent> {
        // Try direct price/size format first: {"market":"...","asset_id":"...","price":"0.89","size":"..."}
        if let (Some(asset_id), Some(price_str)) = (
            value.get("asset_id").and_then(|v| v.as_str()),
            value.get("price").and_then(|v| v.as_str())
        ) {
            if let Some(condition_id) = token_to_market.get(asset_id) {
                if let Ok(price_val) = price_str.parse::<f64>() {
                    let spread = 0.01;
                    let yes_bid = (price_val - spread / 2.0).max(0.001);
                    let yes_ask = (price_val + spread / 2.0).min(0.999);
                    let no_bid = (1.0 - yes_ask).max(0.001);
                    let no_ask = (1.0 - yes_bid).max(0.001);
                    let now = chrono::Utc::now().timestamp();
                    return Some(MarketEvent::new(
                        condition_id.clone(),
                        yes_bid,
                        yes_ask,
                        no_bid,
                        no_ask,
                        now,
                    ));
                }
            }
        }

        // Polymarket CLOB WebSocket format with bids/asks arrays
        let token_id = value.get("token_id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("asset_id").and_then(|v| v.as_str()))?;

        let condition_id = token_to_market.get(token_id)?;

        // Try to extract bids/asks
        let bids = value.get("bids").and_then(|v| v.as_array())?;
        let asks = value.get("asks").and_then(|v| v.as_array())?;

        if bids.is_empty() || asks.is_empty() {
            return None;
        }

        // Get best bid and ask
        let best_bid = bids.first()?;
        let best_ask = asks.first()?;

        let yes_bid = best_bid.get("price")?.as_f64()?;
        let yes_ask = best_ask.get("price")?.as_f64()?;

        let ts = value.get("timestamp")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        // NO = 1 - YES
        let no_bid = (1.0 - yes_ask).max(0.001);
        let no_ask = (1.0 - yes_bid).max(0.001);

        Some(MarketEvent::new(
            condition_id.clone(),
            yes_bid,
            yes_ask,
            no_bid,
            no_ask,
            ts,
        ))
    }

    /// Receive the next event (blocking)
    pub async fn recv(&mut self) -> Option<MarketEvent> {
        self.event_rx.recv().await
    }

    /// Try to receive an event (non-blocking)
    pub fn try_recv(&mut self) -> Option<MarketEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Get all subscribed markets
    pub async fn markets(&self) -> Vec<MarketSubscription> {
        let state = self.state.lock().await;
        state.markets.values().cloned().collect()
    }

    /// Get a specific market subscription
    pub async fn get_market(&self, condition_id: &str) -> Option<MarketSubscription> {
        let state = self.state.lock().await;
        state.markets.get(condition_id).cloned()
    }

    /// Get the book state for a market
    pub async fn get_book_snapshot(&self, condition_id: &str) -> Option<(MarketSnapshot, MarketSnapshot)> {
        let state = self.state.lock().await;
        let book_state = state.book_states.get(condition_id)?;
        let snapshot = book_state.snapshot()?;
        Some((snapshot.yes, snapshot.no))
    }

    /// Add a new market subscription
    pub async fn add_subscription(&mut self, subscription: MarketSubscription) -> Result<()> {
        let mut state = self.state.lock().await;

        state.token_to_market.insert(subscription.yes_token_id.clone(), subscription.condition_id.clone());
        state.token_to_market.insert(subscription.no_token_id.clone(), subscription.condition_id.clone());
        state.markets.insert(subscription.condition_id.clone(), subscription.clone());
        state.book_states.insert(subscription.condition_id.clone(), DualBookState::default());

        Ok(())
    }

    /// Remove a market subscription
    pub async fn remove_subscription(&mut self, condition_id: &str) -> Result<()> {
        let mut state = self.state.lock().await;

        if let Some(market) = state.markets.remove(condition_id) {
            state.token_to_market.remove(&market.yes_token_id);
            state.token_to_market.remove(&market.no_token_id);
        }
        state.book_states.remove(condition_id);

        Ok(())
    }

    /// Get feed statistics
    pub async fn stats(&self) -> FeedStats {
        let state = self.state.lock().await;
        FeedStats {
            num_markets: state.markets.len(),
            num_tokens: state.token_to_market.len(),
        }
    }
}

/// Feed statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedStats {
    pub num_markets: usize,
    pub num_tokens: usize,
}

/// Multi-market event aggregator
///
/// Collects events from multiple markets and provides aggregated views
pub struct MultiMarketAggregator {
    /// Latest event for each market
    latest_events: Arc<RwLock<HashMap<String, MarketEvent>>>,
    /// Market subscriptions
    subscriptions: Arc<RwLock<HashMap<String, MarketSubscription>>>,
}

impl MultiMarketAggregator {
    /// Create a new aggregator
    pub fn new() -> Self {
        Self {
            latest_events: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update with a new event
    pub async fn update(&self, event: MarketEvent) {
        let mut events = self.latest_events.write().await;
        events.insert(event.condition_id.clone(), event);
    }

    /// Get the latest event for a market
    pub async fn get_latest(&self, condition_id: &str) -> Option<MarketEvent> {
        let events = self.latest_events.read().await;
        events.get(condition_id).cloned()
    }

    /// Get all latest events
    pub async fn get_all_latest(&self) -> HashMap<String, MarketEvent> {
        let events = self.latest_events.read().await;
        events.clone()
    }

    /// Get events for a specific timeframe
    pub async fn get_by_timeframe(&self, timeframe: Timeframe) -> Vec<MarketEvent> {
        let subs = self.subscriptions.read().await;
        let events = self.latest_events.read().await;

        subs.values()
            .filter(|sub| sub.timeframe == timeframe)
            .filter_map(|sub| events.get(&sub.condition_id).cloned())
            .collect()
    }

    /// Add a market subscription
    pub async fn add_subscription(&self, subscription: MarketSubscription) {
        let mut subs = self.subscriptions.write().await;
        subs.insert(subscription.condition_id.clone(), subscription);
    }

    /// Find markets with large edges
    pub async fn find_edge_opportunities(
        &self,
        min_edge: f64,
    ) -> Vec<(String, f64, Direction)> {
        let events = self.latest_events.read().await;
        let mut opportunities = Vec::new();

        for (condition_id, event) in events.iter() {
            if !event.is_valid() {
                continue;
            }

            // Simple edge detection: if yes_mid is far from 0.5
            let yes_edge = (event.yes_mid - 0.5).abs();

            if yes_edge >= min_edge {
                let direction = if event.yes_mid > 0.5 {
                    Direction::Yes
                } else {
                    Direction::No
                };
                opportunities.push((condition_id.clone(), yes_edge, direction));
            }
        }

        opportunities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        opportunities
    }
}

impl Default for MultiMarketAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_event_new() {
        let event = MarketEvent::new(
            "test-1".to_string(),
            0.49,
            0.51,
            0.49,
            0.51,
            1000,
        );

        assert_eq!(event.condition_id, "test-1");
        assert!((event.yes_mid - 0.5).abs() < f64::EPSILON);
        assert_eq!(event.ts, 1000);
    }

    #[test]
    fn test_market_event_is_valid() {
        let valid = MarketEvent::new("test".to_string(), 0.49, 0.51, 0.49, 0.51, 1000);
        assert!(valid.is_valid());

        let invalid = MarketEvent::new("test".to_string(), 0.0, 0.51, 0.49, 0.51, 1000);
        assert!(!invalid.is_valid());

        let invalid2 = MarketEvent::new("test".to_string(), 0.51, 0.49, 0.49, 0.51, 1000);
        assert!(!invalid2.is_valid()); // ask < bid
    }

    #[test]
    fn test_market_event_spread() {
        let event = MarketEvent::new("test".to_string(), 0.49, 0.51, 0.49, 0.51, 1000);

        assert!((event.yes_spread() - 0.02).abs() < f64::EPSILON);
        assert!((event.no_spread() - 0.02).abs() < f64::EPSILON);
    }

    #[test]
    fn test_market_event_is_broken_book() {
        let ok = MarketEvent::new("test".to_string(), 0.49, 0.51, 0.49, 0.51, 1000);
        assert!(!ok.is_broken_book(0.01));

        let broken = MarketEvent::new("test".to_string(), 0.50, 0.60, 0.50, 0.60, 1000);
        assert!(broken.is_broken_book(0.01));
    }

    #[test]
    fn test_multi_market_feed_state_default() {
        let state = MultiMarketFeedState::default();
        assert_eq!(state.markets.len(), 0);
        assert_eq!(state.book_states.len(), 0);
        assert_eq!(state.token_to_market.len(), 0);
    }

    #[tokio::test]
    async fn test_multi_market_aggregator() {
        let aggregator = MultiMarketAggregator::new();

        let event = MarketEvent::new(
            "test-1".to_string(),
            0.49,
            0.51,
            0.49,
            0.51,
            1000,
        );

        aggregator.update(event).await;

        let retrieved = aggregator.get_latest("test-1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().condition_id, "test-1");
    }

    #[tokio::test]
    async fn test_multi_market_aggregator_add_subscription() {
        let aggregator = MultiMarketAggregator::new();

        let sub = MarketSubscription {
            condition_id: "test-1".to_string(),
            yes_token_id: "yes-1".to_string(),
            no_token_id: "no-1".to_string(),
            timeframe: Timeframe::M5,
            start_time: 1000,
            end_time: 1300,
            strike_price: 71000.0,
        };

        aggregator.add_subscription(sub).await;

        let subs = aggregator.subscriptions.read().await;
        assert_eq!(subs.len(), 1);
        assert!(subs.contains_key("test-1"));
    }

    #[tokio::test]
    async fn test_find_edge_opportunities() {
        let aggregator = MultiMarketAggregator::new();

        // Add some events
        aggregator.update(MarketEvent::new(
            "high-edge".to_string(),
            0.40,
            0.42,
            0.58,
            0.60,
            1000,
        )).await;

        aggregator.update(MarketEvent::new(
            "low-edge".to_string(),
            0.49,
            0.51,
            0.49,
            0.51,
            1000,
        )).await;

        let opportunities = aggregator.find_edge_opportunities(0.08).await;

        assert_eq!(opportunities.len(), 1);
        assert_eq!(opportunities[0].0, "high-edge");
        assert!((opportunities[0].1 - 0.08).abs() < 0.01);
    }
}
