use anyhow::Result;
use clap::ValueEnum;
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::types::{Decimal, U256};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::bot::discovery::fetch_snapshot;
use crate::bot::logging::JsonlEventLogger;

const CLOB_MARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ReplayMode {
    #[value(alias = "event")]
    EventByEvent,
    #[value(alias = "live-parity")]
    LiveParity1s,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum LiveFeedMode {
    Poll,
    Websocket,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum OutcomeSide {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum BookChangeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize)]
pub struct BookDeltaEvent {
    pub market_id: Option<String>,
    pub token_id: String,
    pub side: OutcomeSide,
    pub ts_exchange: f64,
    pub best_bid: f64,
    pub best_ask: f64,
    pub change_price: Option<f64>,
    pub change_size: Option<f64>,
    pub change_side: Option<BookChangeSide>,
    pub top5_bid_depth: Option<f64>,
    pub top5_ask_depth: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketSnapshot {
    pub midpoint: Option<Decimal>,
    pub best_bid: Option<Decimal>,
    pub best_ask: Option<Decimal>,
    pub spread: Option<Decimal>,
    pub top5_bid_depth: Decimal,
    pub top5_ask_depth: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct DualSnapshot {
    pub yes: MarketSnapshot,
    pub no: MarketSnapshot,
    pub ts_exchange: f64,
}

#[derive(Debug, Clone)]
struct TokenBookState {
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    top5_bid_depth: Option<f64>,
    top5_ask_depth: Option<f64>,
    last_ts_exchange: f64,
}

impl Default for TokenBookState {
    fn default() -> Self {
        Self {
            best_bid: None,
            best_ask: None,
            top5_bid_depth: None,
            top5_ask_depth: None,
            last_ts_exchange: 0.0,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct DualBookState {
    yes: TokenBookState,
    no: TokenBookState,
}

impl DualBookState {
    pub fn apply(&mut self, event: &BookDeltaEvent) {
        let state = match event.side {
            OutcomeSide::Yes => &mut self.yes,
            OutcomeSide::No => &mut self.no,
        };

        state.best_bid = Some(event.best_bid);
        state.best_ask = Some(event.best_ask);
        if let Some(depth) = event.top5_bid_depth {
            state.top5_bid_depth = Some(depth);
        }
        if let Some(depth) = event.top5_ask_depth {
            state.top5_ask_depth = Some(depth);
        }
        state.last_ts_exchange = event.ts_exchange;
    }

    pub fn snapshot(&self) -> Option<DualSnapshot> {
        let yes_bid = self.yes.best_bid?;
        let yes_ask = self.yes.best_ask?;
        let no_bid = self.no.best_bid?;
        let no_ask = self.no.best_ask?;

        Some(DualSnapshot {
            yes: MarketSnapshot::from_state(
                yes_bid,
                yes_ask,
                self.yes.top5_bid_depth.unwrap_or(0.0),
                self.yes.top5_ask_depth.unwrap_or(0.0),
            ),
            no: MarketSnapshot::from_state(
                no_bid,
                no_ask,
                self.no.top5_bid_depth.unwrap_or(0.0),
                self.no.top5_ask_depth.unwrap_or(0.0),
            ),
            ts_exchange: self.yes.last_ts_exchange.max(self.no.last_ts_exchange),
        })
    }
}

impl MarketSnapshot {
    pub fn from_state(best_bid: f64, best_ask: f64, top5_bid_depth: f64, top5_ask_depth: f64) -> Self {
        let midpoint = (best_bid + best_ask) / 2.0;
        let spread = best_ask - best_bid;
        Self {
            midpoint: Decimal::from_f64_retain(midpoint),
            best_bid: Decimal::from_f64_retain(best_bid),
            best_ask: Decimal::from_f64_retain(best_ask),
            spread: Decimal::from_f64_retain(spread),
            top5_bid_depth: Decimal::from_f64_retain(top5_bid_depth).unwrap_or(Decimal::ZERO),
            top5_ask_depth: Decimal::from_f64_retain(top5_ask_depth).unwrap_or(Decimal::ZERO),
        }
    }
}

pub trait StrategyInputSource {
    fn next_snapshot<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<DualSnapshot>>>;
    fn current_time(&self) -> Option<u64>;
}

pub struct PollingSnapshotSource<'a> {
    client: &'a clob::Client,
    yes_token_id: U256,
    no_token_id: U256,
    last_ts: Option<u64>,
}

impl<'a> PollingSnapshotSource<'a> {
    #[must_use]
    pub fn new(client: &'a clob::Client, yes_token_id: U256, no_token_id: U256) -> Self {
        Self {
            client,
            yes_token_id,
            no_token_id,
            last_ts: None,
        }
    }
}

impl StrategyInputSource for PollingSnapshotSource<'_> {
    fn next_snapshot<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<DualSnapshot>>> {
        Box::pin(async move {
            let yes_snapshot = fetch_snapshot(self.client, self.yes_token_id).await?;
            let no_snapshot = fetch_snapshot(self.client, self.no_token_id).await?;
            let ts = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
            self.last_ts = Some(ts.floor() as u64);
            Ok(Some(DualSnapshot {
                yes: yes_snapshot,
                no: no_snapshot,
                ts_exchange: ts,
            }))
        })
    }

    fn current_time(&self) -> Option<u64> {
        self.last_ts
    }
}

pub struct MarketWebsocketFeed {
    rx: mpsc::UnboundedReceiver<BookDeltaEvent>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl MarketWebsocketFeed {
    pub async fn connect(
        market_id: Option<String>,
        yes_token_id: U256,
        no_token_id: U256,
        logger: Option<JsonlEventLogger>,
    ) -> Result<Self> {
        let yes = yes_token_id.to_string();
        let no = no_token_id.to_string();
        let (tx, rx) = mpsc::unbounded_channel();

        let join_handle = tokio::spawn(async move {
            loop {
                let stream = connect_async(CLOB_MARKET_WS_URL).await;
                let Ok((ws_stream, _)) = stream else {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                };

                let (mut write, mut read) = ws_stream.split();
                let subscribe = serde_json::json!({
                    "type": "market",
                    "assets_ids": [yes, no]
                });

                if write.send(Message::Text(subscribe.to_string().into())).await.is_err() {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }

                while let Some(message) = read.next().await {
                    let Ok(message) = message else {
                        break;
                    };

                    let payload = match message {
                        Message::Text(text) => text.to_string(),
                        Message::Binary(bin) => String::from_utf8_lossy(&bin).to_string(),
                        Message::Ping(_) | Message::Pong(_) => continue,
                        Message::Close(_) => break,
                        Message::Frame(_) => continue,
                    };

                    let parsed: serde_json::Result<Value> = serde_json::from_str(&payload);
                    let Ok(value) = parsed else {
                        continue;
                    };

                    for event in parse_market_ws_value(&value, market_id.as_deref(), &yes, &no) {
                        if let Some(logger) = &logger {
                            logger.log("raw_book_events", &event);
                        }
                        if tx.send(event).is_err() {
                            return;
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        Ok(Self { rx, join_handle })
    }

    pub async fn recv(&mut self) -> Option<BookDeltaEvent> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<BookDeltaEvent> {
        self.rx.try_recv().ok()
    }

    pub async fn shutdown(self) {
        self.join_handle.abort();
        let _ = self.join_handle.await;
    }
}

pub struct WebsocketSnapshotSource {
    feed: MarketWebsocketFeed,
    book_state: DualBookState,
    last_ts: Option<u64>,
}

impl WebsocketSnapshotSource {
    pub async fn connect(
        market_id: Option<String>,
        yes_token_id: U256,
        no_token_id: U256,
        logger: Option<JsonlEventLogger>,
    ) -> Result<Self> {
        Ok(Self {
            feed: MarketWebsocketFeed::connect(market_id, yes_token_id, no_token_id, logger).await?,
            book_state: DualBookState::default(),
            last_ts: None,
        })
    }

    pub async fn shutdown(self) {
        self.feed.shutdown().await;
    }

    /// Wait for the NEXT single WebSocket event, apply it, return snapshot.
    /// True event-by-event processing — blocks until an event arrives.
    pub async fn recv_next(&mut self) -> Option<DualSnapshot> {
        let event = self.feed.recv().await?;
        self.book_state.apply(&event);
        self.last_ts = Some(event.ts_exchange.floor() as u64);
        self.book_state.snapshot()
    }

    /// Drain all pending events, process each through callback, return final snapshot.
    /// Use this to drain backlog between strategy processing.
    pub fn try_recv_all<F: FnMut(DualSnapshot)>(&mut self, mut on_snapshot: F) -> Option<DualSnapshot> {
        let mut last = None;
        while let Some(event) = self.feed.try_recv() {
            self.book_state.apply(&event);
            self.last_ts = Some(event.ts_exchange.floor() as u64);
            if let Some(snap) = self.book_state.snapshot() {
                on_snapshot(snap.clone());
                last = Some(snap);
            }
        }
        last
    }
}

impl StrategyInputSource for WebsocketSnapshotSource {
    fn next_snapshot<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<DualSnapshot>>> {
        Box::pin(async move {
            while let Some(event) = self.feed.try_recv() {
                self.book_state.apply(&event);
            }
            let snapshot = self.book_state.snapshot();
            self.last_ts = snapshot.as_ref().map(|item| item.ts_exchange.floor() as u64);
            Ok(snapshot)
        })
    }

    fn current_time(&self) -> Option<u64> {
        self.last_ts
    }
}

pub enum LiveStrategyInputSource<'a> {
    Poll(PollingSnapshotSource<'a>),
    Websocket(WebsocketSnapshotSource),
}

impl<'a> LiveStrategyInputSource<'a> {
    #[must_use]
    pub fn poll(client: &'a clob::Client, yes_token_id: U256, no_token_id: U256) -> Self {
        Self::Poll(PollingSnapshotSource::new(client, yes_token_id, no_token_id))
    }

    pub async fn websocket(
        market_id: Option<String>,
        yes_token_id: U256,
        no_token_id: U256,
        logger: Option<JsonlEventLogger>,
    ) -> Result<Self> {
        Ok(Self::Websocket(
            WebsocketSnapshotSource::connect(market_id, yes_token_id, no_token_id, logger).await?,
        ))
    }

    pub async fn shutdown(self) {
        if let Self::Websocket(source) = self {
            source.shutdown().await;
        }
    }

    pub fn is_websocket(&self) -> bool {
        matches!(self, Self::Websocket(_))
    }

    /// Event-by-event: wait for next WebSocket event and return snapshot.
    /// Returns None if Poll variant (call next_snapshot instead).
    pub async fn recv_next(&mut self) -> Option<DualSnapshot> {
        match self {
            Self::Websocket(source) => source.recv_next().await,
            Self::Poll(_) => None,
        }
    }

    /// Drain pending WebSocket events, process each through callback, return final snapshot.
    /// Returns None if Poll variant.
    pub fn try_recv_all<F: FnMut(DualSnapshot)>(&mut self, on_snapshot: F) -> Option<DualSnapshot> {
        match self {
            Self::Websocket(source) => source.try_recv_all(on_snapshot),
            Self::Poll(_) => None,
        }
    }
}

impl StrategyInputSource for LiveStrategyInputSource<'_> {
    fn next_snapshot<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<DualSnapshot>>> {
        match self {
            Self::Poll(source) => source.next_snapshot(),
            Self::Websocket(source) => source.next_snapshot(),
        }
    }

    fn current_time(&self) -> Option<u64> {
        match self {
            Self::Poll(source) => source.current_time(),
            Self::Websocket(source) => source.current_time(),
        }
    }
}

pub struct ReplaySnapshotSource {
    snapshots: VecDeque<DualSnapshot>,
    current_ts: Option<u64>,
}

impl ReplaySnapshotSource {
    #[must_use]
    pub fn new(snapshots: Vec<DualSnapshot>) -> Self {
        Self {
            snapshots: snapshots.into(),
            current_ts: None,
        }
    }
}

impl StrategyInputSource for ReplaySnapshotSource {
    fn next_snapshot<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<DualSnapshot>>> {
        Box::pin(async move {
            let next = self.snapshots.pop_front();
            self.current_ts = next.as_ref().map(|snapshot| snapshot.ts_exchange.floor() as u64);
            Ok(next)
        })
    }

    fn current_time(&self) -> Option<u64> {
        self.current_ts
    }
}

pub fn parse_market_ws_value(
    value: &Value,
    market_id: Option<&str>,
    yes_token_id: &str,
    no_token_id: &str,
) -> Vec<BookDeltaEvent> {
    match value {
        Value::Array(items) => items
            .iter()
            .flat_map(|item| parse_market_ws_value(item, market_id, yes_token_id, no_token_id))
            .collect(),
        Value::Object(map) => parse_market_ws_object(map, market_id, yes_token_id, no_token_id),
        _ => Vec::new(),
    }
}

fn parse_market_ws_object(
    object: &serde_json::Map<String, Value>,
    market_id: Option<&str>,
    yes_token_id: &str,
    no_token_id: &str,
) -> Vec<BookDeltaEvent> {
    let event_type = object
        .get("event_type")
        .and_then(Value::as_str)
        .or_else(|| object.get("type").and_then(Value::as_str))
        .unwrap_or_default();

    match event_type {
        "book" => parse_book_message(object, market_id, yes_token_id, no_token_id).into_iter().collect(),
        "price_change" => parse_price_change_message(object, market_id, yes_token_id, no_token_id)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_book_message(
    object: &serde_json::Map<String, Value>,
    market_id: Option<&str>,
    yes_token_id: &str,
    no_token_id: &str,
) -> Option<BookDeltaEvent> {
    let token_id = object
        .get("asset_id")
        .and_then(Value::as_str)
        .or_else(|| object.get("token_id").and_then(Value::as_str))
        .or_else(|| object.get("market").and_then(Value::as_str))?;
    let side = outcome_side_from_token(token_id, yes_token_id, no_token_id)?;

    let buys = object
        .get("buys")
        .or_else(|| object.get("bids"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let sells = object
        .get("sells")
        .or_else(|| object.get("asks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let best_bid = extract_best_price(&buys, true)?;
    let best_ask = extract_best_price(&sells, false)?;
    let top5_bid_depth = sum_depth(&buys, 5, true);
    let top5_ask_depth = sum_depth(&sells, 5, false);
    let ts_exchange = object
        .get("timestamp")
        .and_then(as_f64)
        .unwrap_or_else(current_ts_seconds);

    Some(BookDeltaEvent {
        market_id: market_id.map(ToOwned::to_owned),
        token_id: token_id.to_string(),
        side,
        ts_exchange,
        best_bid,
        best_ask,
        change_price: None,
        change_size: None,
        change_side: None,
        top5_bid_depth: Some(top5_bid_depth),
        top5_ask_depth: Some(top5_ask_depth),
        source: "websocket_book",
    })
}

fn parse_price_change_message(
    object: &serde_json::Map<String, Value>,
    market_id: Option<&str>,
    yes_token_id: &str,
    no_token_id: &str,
) -> Option<BookDeltaEvent> {
    let token_id = object
        .get("asset_id")
        .and_then(Value::as_str)
        .or_else(|| object.get("token_id").and_then(Value::as_str))?;
    let side = outcome_side_from_token(token_id, yes_token_id, no_token_id)?;

    let best_bid = object.get("best_bid").and_then(as_f64)?;
    let best_ask = object.get("best_ask").and_then(as_f64)?;
    let ts_exchange = object
        .get("timestamp")
        .and_then(as_f64)
        .unwrap_or_else(current_ts_seconds);

    Some(BookDeltaEvent {
        market_id: market_id.map(ToOwned::to_owned),
        token_id: token_id.to_string(),
        side,
        ts_exchange,
        best_bid,
        best_ask,
        change_price: object.get("change_price").and_then(as_f64),
        change_size: object.get("change_size").and_then(as_f64),
        change_side: object
            .get("change_side")
            .and_then(Value::as_str)
            .and_then(parse_change_side),
        top5_bid_depth: None,
        top5_ask_depth: None,
        source: "websocket_price_change",
    })
}

fn outcome_side_from_token(token_id: &str, yes_token_id: &str, no_token_id: &str) -> Option<OutcomeSide> {
    if token_id == yes_token_id {
        Some(OutcomeSide::Yes)
    } else if token_id == no_token_id {
        Some(OutcomeSide::No)
    } else {
        None
    }
}

fn parse_change_side(raw: &str) -> Option<BookChangeSide> {
    match raw.to_ascii_uppercase().as_str() {
        "BUY" => Some(BookChangeSide::Buy),
        "SELL" => Some(BookChangeSide::Sell),
        _ => None,
    }
}

fn extract_best_price(levels: &[Value], is_bid: bool) -> Option<f64> {
    let mut prices = Vec::new();
    for level in levels {
        let price = match level {
            Value::Object(map) => map.get("price").and_then(as_f64),
            Value::Array(values) => values.first().and_then(as_f64),
            _ => None,
        };
        if let Some(price) = price {
            prices.push(price);
        }
    }
    if prices.is_empty() {
        None
    } else if is_bid {
        prices.into_iter().reduce(f64::max)
    } else {
        prices.into_iter().reduce(f64::min)
    }
}

fn sum_depth(levels: &[Value], top_n: usize, descending: bool) -> f64 {
    let mut parsed = Vec::new();
    for level in levels {
        let (price, size) = match level {
            Value::Object(map) => (
                map.get("price").and_then(as_f64),
                map.get("size").and_then(as_f64),
            ),
            Value::Array(values) if values.len() >= 2 => (as_f64(&values[0]), as_f64(&values[1])),
            _ => (None, None),
        };
        if let (Some(price), Some(size)) = (price, size) {
            parsed.push((price, size.max(0.0)));
        }
    }
    parsed.sort_by(|left, right| {
        if descending {
            right.0.partial_cmp(&left.0).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    parsed.into_iter().take(top_n).map(|(_, size)| size).sum()
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn current_ts_seconds() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}
