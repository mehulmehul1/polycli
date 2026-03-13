# Execution Specification

## 1. Goal

Define the **execution layer** for Polymarket trading that:

1. Converts `StrategyDecision` to live orders
2. Handles partial fills (FAK) vs full fills (FOK)
3. Confirms fills via user channel before position tracking
4. Handles stale feeds with exit-only behavior
5. Provides deterministic replay for backtest

## 2. Current State

### 2.1 Execution flow

| Step | Current Code | Location |
|------|-------------|----------|
| Signal generation | `strategy_runner.rs` | `run_shadow_strategy_step` |
| Position tracking | `shadow.rs` | `ShadowPosition` |
| Order placement | `execution.rs` | `execute_order` |
| Fill confirmation | WebSocket feed | CLOB SDK |
| Settlement | `settlement.rs` | post-market |

### 2.2 Problems with current approach

1. **Shadow vs live divergence** — `ShadowPosition` tracks paper trades, live orders tracked separately
2. **No FAK support** — All orders are FOK or GTC, partial fills not handled
3. **Fill confirmation lag** — Position updates before confirmation, can over-trade
4. **No stale-feed handling** — Continues trading on old data
5. **No user-channel flow** — Not applicable to Polymarket (not hyperliquid)

## 3. Execution Architecture

### 3.1 Core types

```rust
// src/bot/execution/types.rs

/// Order type for execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Fill-or-Kill: must fill completely or cancel
    FOK,
    
    /// Fill-and-Kill (Immediate-or-Cancel): partial fills allowed, remainder cancelled
    FAK,
    
    /// Good-Til-Cancelled: stays on book until filled or cancelled
    GTC,
    
    /// Good-Til-Date: stays until time expires
    GTD { expiry_ts: u64 },
}

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Order request from strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub market_slug: String,
    pub condition_id: String,
    pub direction: Direction,
    pub side: OrderSide,
    pub price: f64,
    pub size_usd: f64,
    pub order_type: OrderType,
    pub reason: EntryReason,
    pub ts: u64,
}

/// Fill result from exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    pub order_id: String,
    pub market_slug: String,
    pub direction: Direction,
    pub side: OrderSide,
    pub filled_price: f64,
    pub filled_size_usd: f64,
    pub remaining_size_usd: f64,
    pub fee_usd: f64,
    pub ts: u64,
    pub is_complete: bool,
}

/// Position after fill confirmation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmedPosition {
    pub condition_id: String,
    pub direction: Direction,
    pub entry_ts: u64,
    pub entry_price: f64,
    pub size_usd: f64,
    pub entry_order_id: String,
    pub entry_reason: EntryReason,
    pub current_value_usd: f64,
    pub unrealized_pnl_usd: f64,
}

/// Execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Paper trading (no real orders)
    Shadow,
    
    /// Dry run (submit to API but don't execute)
    DryRun,
    
    /// Live execution
    Live,
}

/// Feed health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedHealth {
    Healthy,
    Stale { age_s: u64 },
    Disconnected,
}

impl FeedHealth {
    pub fn is_healthy(&self, max_stale_s: u64) -> bool {
        match self {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => *age_s <= max_stale_s,
            FeedHealth::Disconnected => false,
        }
    }
    
    pub fn is_tradeable(&self, market_duration_s: i64) -> bool {
        match self {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => {
                // For short markets, any staleness is unacceptable
                market_duration_s > 300 || *age_s < 2
            }
            FeedHealth::Disconnected => false,
        }
    }
}
```

### 3.2 Execution engine

```rust
// src/bot/execution/engine.rs

/// Execution engine converts strategy decisions to orders
pub struct ExecutionEngine {
    config: ExecutionConfig,
    position_tracker: PositionTracker,
    order_client: Arc<dyn OrderClient>,
    feed_monitor: FeedMonitor,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionConfig {
    /// Default order type for entries
    pub entry_order_type: OrderType,
    
    /// Default order type for exits
    pub exit_order_type: OrderType,
    
    /// Maximum staleness before blocking entries (seconds)
    pub max_feed_stale_s: u64,
    
    /// For markets <= this duration, require fresh feed
    pub short_market_threshold_s: i64,
    
    /// Require fill confirmation before position update
    pub require_fill_confirmation: bool,
    
    /// Maximum time to wait for fill confirmation
    pub fill_confirmation_timeout_s: u64,
    
    /// Slippage tolerance for market orders
    pub slippage_tolerance: f64,
    
    /// Execution mode
    pub mode: ExecutionMode,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            entry_order_type: OrderType::FAK,
            exit_order_type: OrderType::FAK,
            max_feed_stale_s: 5,
            short_market_threshold_s: 300, // 5 minutes
            require_fill_confirmation: true,
            fill_confirmation_timeout_s: 5,
            slippage_tolerance: 0.02,
            mode: ExecutionMode::Shadow,
        }
    }
}

impl ExecutionEngine {
    /// Process strategy decision and execute if needed
    pub async fn on_decision(
        &mut self,
        decision: &StrategyDecision,
        obs: &StrategyObservation,
    ) -> Option<ExecutionResult> {
        match decision {
            StrategyDecision::Hold => None,
            
            StrategyDecision::Block { reason } => {
                self.log_block(reason, obs);
                None
            }
            
            StrategyDecision::Enter { direction, reason, suggested_size_usd } => {
                self.execute_entry(*direction, reason, *suggested_size_usd, obs).await
            }
            
            StrategyDecision::Exit { reason, pnl_estimate } => {
                self.execute_exit(reason, *pnl_estimate, obs).await
            }
        }
    }
    
    async fn execute_entry(
        &mut self,
        direction: Direction,
        reason: &EntryReason,
        size_usd: f64,
        obs: &StrategyObservation,
    ) -> Option<ExecutionResult> {
        // 1. Check feed health
        let feed_health = self.feed_monitor.health(&obs.asset);
        if !feed_health.is_tradeable(obs.market_end_ts - obs.ts as i64) {
            return Some(ExecutionResult::Blocked {
                reason: format!("feed_stale: {:?}", feed_health),
            });
        }
        
        // 2. Check for existing position
        if self.position_tracker.has_position(&obs.condition_id) {
            return Some(ExecutionResult::Blocked {
                reason: "already_in_position".to_string(),
            });
        }
        
        // 3. Determine entry price
        let (side, price) = match direction {
            Direction::Yes => (OrderSide::Buy, obs.yes_ask),
            Direction::No => (OrderSide::Buy, obs.no_ask),
        };
        
        // Add slippage for FAK
        let price_with_slippage = match self.config.entry_order_type {
            OrderType::FAK | OrderType::FOK => price + self.config.slippage_tolerance,
            OrderType::GTC | OrderType::GTD { .. } => price,
        };
        
        // 4. Create order request
        let request = OrderRequest {
            market_slug: obs.market_slug.clone(),
            condition_id: obs.condition_id.clone(),
            direction,
            side,
            price: price_with_slippage,
            size_usd,
            order_type: self.config.entry_order_type,
            reason: reason.clone(),
            ts: obs.ts,
        };
        
        // 5. Execute based on mode
        match self.config.mode {
            ExecutionMode::Shadow => {
                // Paper trade: assume full fill at mid
                let fill = FillResult {
                    order_id: format!("shadow_{}", obs.ts),
                    market_slug: obs.market_slug.clone(),
                    direction,
                    side,
                    filled_price: (obs.yes_bid + obs.yes_ask) / 2.0,
                    filled_size_usd: size_usd,
                    remaining_size_usd: 0.0,
                    fee_usd: size_usd * 0.02, // 2% fee estimate
                    ts: obs.ts,
                    is_complete: true,
                };
                
                let position = self.position_tracker.record_entry(fill, reason.clone());
                Some(ExecutionResult::Entered { position, fill })
            }
            
            ExecutionMode::DryRun => {
                // Log order but don't submit
                self.log_order(&request, "dry_run");
                Some(ExecutionResult::DryRun { request })
            }
            
            ExecutionMode::Live => {
                // Submit to exchange and wait for confirmation
                let result = self.order_client.submit(&request).await;
                
                match result {
                    Ok(fill) => {
                        if fill.is_complete || fill.filled_size_usd > 0.0 {
                            let position = self.position_tracker.record_entry(fill.clone(), reason.clone());
                            Some(ExecutionResult::Entered { position, fill })
                        } else {
                            Some(ExecutionResult::NoFill { request })
                        }
                    }
                    Err(e) => Some(ExecutionResult::Error {
                        request,
                        error: e.to_string(),
                    }),
                }
            }
        }
    }
    
    async fn execute_exit(
        &mut self,
        reason: &ExitReason,
        pnl_estimate: f64,
        obs: &StrategyObservation,
    ) -> Option<ExecutionResult> {
        // 1. Get current position
        let position = self.position_tracker.get_position(&obs.condition_id)?;
        
        // 2. Determine exit price
        let (side, price) = match position.direction {
            Direction::Yes => (OrderSide::Sell, obs.yes_bid),
            Direction::No => (OrderSide::Sell, obs.no_bid),
        };
        
        // 3. Create exit order
        let request = OrderRequest {
            market_slug: obs.market_slug.clone(),
            condition_id: obs.condition_id.clone(),
            direction: position.direction,
            side,
            price,
            size_usd: position.size_usd,
            order_type: self.config.exit_order_type,
            reason: EntryReason {
                source: SignalSource::Indicators, // placeholder
                confidence: Confidence::MAX,
                detail: format!("exit: {:?}", reason),
                fair_value_edge: None,
                qlib_score: None,
            },
            ts: obs.ts,
        };
        
        // 4. Execute based on mode
        match self.config.mode {
            ExecutionMode::Shadow => {
                let fill = FillResult {
                    order_id: format!("shadow_exit_{}", obs.ts),
                    market_slug: obs.market_slug.clone(),
                    direction: position.direction,
                    side,
                    filled_price: price,
                    filled_size_usd: position.size_usd,
                    remaining_size_usd: 0.0,
                    fee_usd: position.size_usd * 0.02,
                    ts: obs.ts,
                    is_complete: true,
                };
                
                let realized_pnl = self.position_tracker.record_exit(fill.clone());
                Some(ExecutionResult::Exited {
                    pnl: realized_pnl,
                    fill,
                    reason: reason.clone(),
                })
            }
            
            ExecutionMode::DryRun => {
                self.log_order(&request, "dry_run_exit");
                Some(ExecutionResult::DryRun { request })
            }
            
            ExecutionMode::Live => {
                let result = self.order_client.submit(&request).await;
                
                match result {
                    Ok(fill) => {
                        let realized_pnl = self.position_tracker.record_exit(fill.clone());
                        Some(ExecutionResult::Exited {
                            pnl: realized_pnl,
                            fill,
                            reason: reason.clone(),
                        })
                    }
                    Err(e) => Some(ExecutionResult::Error {
                        request,
                        error: e.to_string(),
                    }),
                }
            }
        }
    }
    
    fn log_block(&self, reason: &str, obs: &StrategyObservation) {
        println!(
            "[BLOCK] ts={} market={} reason={}",
            obs.ts, obs.market_slug, reason
        );
    }
    
    fn log_order(&self, request: &OrderRequest, mode: &str) {
        println!(
            "[{}] ts={} market={} dir={:?} side={:?} price={:.4} size={:.2f}",
            mode,
            request.ts,
            request.market_slug,
            request.direction,
            request.side,
            request.price,
            request.size_usd
        );
    }
}

#[derive(Debug, Clone)]
pub enum ExecutionResult {
    Entered {
        position: ConfirmedPosition,
        fill: FillResult,
    },
    Exited {
        pnl: f64,
        fill: FillResult,
        reason: ExitReason,
    },
    NoFill {
        request: OrderRequest,
    },
    Blocked {
        reason: String,
    },
    DryRun {
        request: OrderRequest,
    },
    Error {
        request: OrderRequest,
        error: String,
    },
}
```

### 3.3 FAK vs FOK handling

```rust
// src/bot/execution/order_client.rs

/// Order client trait for different execution backends
#[async_trait]
pub trait OrderClient: Send + Sync {
    /// Submit order and return fill result
    async fn submit(&self, request: &OrderRequest) -> Result<FillResult, ExecutionError>;
    
    /// Cancel order by ID
    async fn cancel(&self, order_id: &str) -> Result<(), ExecutionError>;
    
    /// Get order status
    async fn status(&self, order_id: &str) -> Result<OrderStatus, ExecutionError>;
}

/// Polymarket CLOB order client
pub struct PolymarketOrderClient {
    clob_client: CLOBClient,
    config: PolymarketConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    /// Use FAK for entries (recommended)
    pub use_fak_entries: bool,
    
    /// Use FAK for exits
    pub use_fak_exits: bool,
    
    /// FOK fallback if FAK fails
    pub fok_fallback: bool,
    
    /// Minimum fill size to accept (USD)
    pub min_fill_usd: f64,
}

#[async_trait]
impl OrderClient for PolymarketOrderClient {
    async fn submit(&self, request: &OrderRequest) -> Result<FillResult, ExecutionError> {
        // 1. Convert to CLOB order
        let clob_order = self.to_clob_order(request)?;
        
        // 2. Submit to CLOB
        let response = self.clob_client.create_order(&clob_order).await?;
        
        // 3. Parse fill result
        let fill = self.parse_fill(response, request)?;
        
        // 4. Handle partial fills
        if !fill.is_complete && fill.filled_size_usd >= self.config.min_fill_usd {
            // Acceptable partial fill
            Ok(fill)
        } else if !fill.is_complete && self.config.fok_fallback {
            // Retry as FOK
            let fok_request = OrderRequest {
                order_type: OrderType::FOK,
                ..request.clone()
            };
            let fok_order = self.to_clob_order(&fok_request)?;
            let fok_response = self.clob_client.create_order(&fok_order).await?;
            self.parse_fill(fok_response, &fok_request)
        } else if !fill.is_complete {
            Err(ExecutionError::PartialFillTooSmall {
                filled: fill.filled_size_usd,
                min: self.config.min_fill_usd,
            })
        } else {
            Ok(fill)
        }
    }
    
    async fn cancel(&self, order_id: &str) -> Result<(), ExecutionError> {
        self.clob_client.cancel_order(order_id).await?;
        Ok(())
    }
    
    async fn status(&self, order_id: &str) -> Result<OrderStatus, ExecutionError> {
        let status = self.clob_client.get_order(order_id).await?;
        Ok(status.into())
    }
}

impl PolymarketOrderClient {
    fn to_clob_order(&self, request: &OrderRequest) -> Result<CLOBOrder, ExecutionError> {
        let token_id = self.get_token_id(&request.market_slug, request.direction)?;
        
        let order_type = match request.order_type {
            OrderType::FOK => "FOK",
            OrderType::FAK => "GTC", // FAK = GTC with immediate execution
            OrderType::GTC => "GTC",
            OrderType::GTD { expiry_ts } => "GTD",
        };
        
        Ok(CLOBOrder {
            token_id,
            side: match request.side {
                OrderSide::Buy => "BUY",
                OrderSide::Sell => "SELL",
            },
            price: request.price,
            size: request.size_usd / request.price, // Convert USD to shares
            order_type: order_type.to_string(),
            expiration: match request.order_type {
                OrderType::GTD { expiry_ts } => Some(expiry_ts),
                _ => None,
            },
        })
    }
    
    fn parse_fill(
        &self,
        response: CLOBOrderResponse,
        request: &OrderRequest,
    ) -> Result<FillResult, ExecutionError> {
        Ok(FillResult {
            order_id: response.order_id,
            market_slug: request.market_slug.clone(),
            direction: request.direction,
            side: request.side,
            filled_price: response.avg_price.unwrap_or(request.price),
            filled_size_usd: response.filled_size.unwrap_or(0.0) * response.avg_price.unwrap_or(request.price),
            remaining_size_usd: response.remaining_size.unwrap_or(0.0) * response.avg_price.unwrap_or(request.price),
            fee_usd: response.fee.unwrap_or(0.0),
            ts: current_timestamp(),
            is_complete: response.status == "FILLED" || response.remaining_size.unwrap_or(0.0) == 0.0,
        })
    }
    
    fn get_token_id(&self, market_slug: &str, direction: Direction) -> Result<String, ExecutionError> {
        // TODO: Look up token ID from market metadata
        // For now, return placeholder
        Ok(format!("{}_{}", market_slug, match direction {
            Direction::Yes => "YES",
            Direction::No => "NO",
        }))
    }
}
```

### 3.4 Fill confirmation flow

```rust
// src/bot/execution/position_tracker.rs

/// Position tracker with fill confirmation
pub struct PositionTracker {
    positions: HashMap<String, ConfirmedPosition>,
    pending_entries: HashMap<String, PendingEntry>,
    pending_exits: HashMap<String, PendingExit>,
    trade_history: Vec<TradeRecord>,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    order_id: String,
    condition_id: String,
    direction: Direction,
    size_usd: f64,
    submitted_ts: u64,
    timeout_ts: u64,
}

#[derive(Debug, Clone)]
struct PendingExit {
    order_id: String,
    condition_id: String,
    size_usd: f64,
    submitted_ts: u64,
    timeout_ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub condition_id: String,
    pub market_slug: String,
    pub direction: Direction,
    pub entry_ts: u64,
    pub entry_price: f64,
    pub entry_size_usd: f64,
    pub exit_ts: Option<u64>,
    pub exit_price: Option<f64>,
    pub exit_size_usd: Option<f64>,
    pub pnl_usd: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub entry_reason: EntryReason,
    pub exit_reason: Option<ExitReason>,
}

impl PositionTracker {
    /// Record entry after fill confirmation
    pub fn record_entry(&mut self, fill: FillResult, reason: EntryReason) -> ConfirmedPosition {
        let position = ConfirmedPosition {
            condition_id: fill.market_slug.clone(), // TODO: proper condition_id
            direction: fill.direction,
            entry_ts: fill.ts,
            entry_price: fill.filled_price,
            size_usd: fill.filled_size_usd,
            entry_order_id: fill.order_id.clone(),
            entry_reason: reason,
            current_value_usd: fill.filled_size_usd, // Will be updated
            unrealized_pnl_usd: 0.0,
        };
        
        self.positions.insert(fill.market_slug.clone(), position.clone());
        
        // Remove from pending
        self.pending_entries.remove(&fill.order_id);
        
        position
    }
    
    /// Record exit after fill confirmation
    pub fn record_exit(&mut self, fill: FillResult) -> f64 {
        if let Some(position) = self.positions.remove(&fill.market_slug) {
            let pnl_usd = (fill.filled_price - position.entry_price) * position.size_usd / position.entry_price;
            let pnl_pct = (fill.filled_price - position.entry_price) / position.entry_price;
            
            let record = TradeRecord {
                condition_id: position.condition_id.clone(),
                market_slug: fill.market_slug.clone(),
                direction: position.direction,
                entry_ts: position.entry_ts,
                entry_price: position.entry_price,
                entry_size_usd: position.size_usd,
                exit_ts: Some(fill.ts),
                exit_price: Some(fill.filled_price),
                exit_size_usd: Some(fill.filled_size_usd),
                pnl_usd: Some(pnl_usd),
                pnl_pct: Some(pnl_pct),
                entry_reason: position.entry_reason,
                exit_reason: None, // Will be filled by caller
            };
            
            self.trade_history.push(record);
            
            // Remove from pending
            self.pending_exits.remove(&fill.order_id);
            
            pnl_usd
        } else {
            0.0
        }
    }
    
    /// Check for timed-out pending orders
    pub fn check_timeouts(&mut self, now_ts: u64) -> Vec<TimeoutResult> {
        let mut results = Vec::new();
        
        // Check pending entries
        let timed_out_entries: Vec<_> = self.pending_entries
            .iter()
            .filter(|(_, pending)| now_ts >= pending.timeout_ts)
            .map(|(order_id, _)| order_id.clone())
            .collect();
        
        for order_id in timed_out_entries {
            self.pending_entries.remove(&order_id);
            results.push(TimeoutResult::EntryTimeout { order_id });
        }
        
        // Check pending exits
        let timed_out_exits: Vec<_> = self.pending_exits
            .iter()
            .filter(|(_, pending)| now_ts >= pending.timeout_ts)
            .map(|(order_id, _)| order_id.clone())
            .collect();
        
        for order_id in timed_out_exits {
            self.pending_exits.remove(&order_id);
            results.push(TimeoutResult::ExitTimeout { order_id });
        }
        
        results
    }
    
    pub fn has_position(&self, condition_id: &str) -> bool {
        self.positions.contains_key(condition_id)
    }
    
    pub fn get_position(&self, condition_id: &str) -> Option<&ConfirmedPosition> {
        self.positions.get(condition_id)
    }
    
    pub fn trade_history(&self) -> &[TradeRecord] {
        &self.trade_history
    }
}

#[derive(Debug, Clone)]
pub enum TimeoutResult {
    EntryTimeout { order_id: String },
    ExitTimeout { order_id: String },
}
```

### 3.5 Stale-feed handling

```rust
// src/bot/execution/feed_monitor.rs

/// Feed health monitor
pub struct FeedMonitor {
    last_update: HashMap<String, u64>,
    config: FeedMonitorConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedMonitorConfig {
    /// Maximum staleness for healthy feed
    pub max_stale_healthy_s: u64,
    
    /// Maximum staleness for tradeable (exit-only mode)
    pub max_stale_tradeable_s: u64,
    
    /// For markets <= this duration, stricter staleness
    pub short_market_threshold_s: i64,
    
    /// For short markets, max staleness
    pub short_market_max_stale_s: u64,
}

impl Default for FeedMonitorConfig {
    fn default() -> Self {
        Self {
            max_stale_healthy_s: 5,
            max_stale_tradeable_s: 30,
            short_market_threshold_s: 300,
            short_market_max_stale_s: 2,
        }
    }
}

impl FeedMonitor {
    pub fn new(config: FeedMonitorConfig) -> Self {
        Self {
            last_update: HashMap::new(),
            config,
        }
    }
    
    /// Record feed update
    pub fn record_update(&mut self, asset: &str, ts: u64) {
        self.last_update.insert(asset.to_string(), ts);
    }
    
    /// Get feed health status
    pub fn health(&self, asset: &str) -> FeedHealth {
        match self.last_update.get(asset) {
            Some(&last_ts) => {
                let now = current_timestamp();
                let age_s = now.saturating_sub(last_ts);
                
                if age_s <= self.config.max_stale_healthy_s {
                    FeedHealth::Healthy
                } else {
                    FeedHealth::Stale { age_s }
                }
            }
            None => FeedHealth::Disconnected,
        }
    }
    
    /// Check if entries are allowed
    pub fn entries_allowed(&self, asset: &str, market_duration_s: i64) -> bool {
        let health = self.health(asset);
        
        match health {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => {
                if market_duration_s <= self.config.short_market_threshold_s {
                    age_s <= self.config.short_market_max_stale_s
                } else {
                    age_s <= self.config.max_stale_healthy_s
                }
            }
            FeedHealth::Disconnected => false,
        }
    }
    
    /// Check if exits are allowed (more permissive)
    pub fn exits_allowed(&self, asset: &str) -> bool {
        let health = self.health(asset);
        
        match health {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => age_s <= self.config.max_stale_tradeable_s,
            FeedHealth::Disconnected => false,
        }
    }
}
```

### 3.6 Execution flow integration

```rust
// src/bot/execution/mod.rs

/// Main execution loop
pub async fn run_execution_loop(
    mut strategy: Box<dyn StrategyEngine>,
    mut execution: ExecutionEngine,
    mut feed: Box<dyn MarketFeed>,
    mut spot_feed: Option<Box<dyn SpotFeed>>,
    config: LoopConfig,
) -> Result<(), ExecutionError> {
    let mut last_ts = 0u64;
    
    loop {
        // 1. Get market snapshot
        let snapshot = feed.snapshot().await?;
        let ts = snapshot.ts;
        
        // Skip duplicate timestamps
        if ts == last_ts {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        last_ts = ts;
        
        // 2. Get spot price if available
        let spot_price = spot_feed.as_mut().and_then(|f| f.current_price(&snapshot.asset));
        
        // 3. Build observation
        let obs = StrategyObservation {
            ts,
            condition_id: snapshot.condition_id.clone(),
            market_slug: snapshot.market_slug.clone(),
            asset: snapshot.asset.clone(),
            duration: snapshot.duration.clone(),
            market_start_ts: snapshot.market_start_ts,
            market_end_ts: snapshot.market_end_ts,
            yes_bid: snapshot.yes_bid,
            yes_ask: snapshot.yes_ask,
            no_bid: snapshot.no_bid,
            no_ask: snapshot.no_ask,
            yes_spread: snapshot.yes_spread,
            no_spread: snapshot.no_spread,
            book_sum: snapshot.yes_ask + snapshot.no_ask,
            book_gap: snapshot.yes_ask + snapshot.no_ask - 1.0,
            yes_mid: (snapshot.yes_bid + snapshot.yes_ask) / 2.0,
            no_mid: (snapshot.no_bid + snapshot.no_ask) / 2.0,
            ind_5s: snapshot.ind_5s,
            ind_1m: snapshot.ind_1m,
            fair_prob_yes: None, // TODO
            fair_prob_calibrated: None,
            qlib_score_yes: None,
            qlib_score_no: None,
            qlib_fresh: false,
            position: execution.position_tracker().get_position(&snapshot.condition_id).cloned(),
            risk_gates: RiskGateState {
                spread_ok: snapshot.yes_spread < 0.08,
                price_range_ok: snapshot.yes_mid > 0.08 && snapshot.yes_mid < 0.92,
                book_integrity_ok: (snapshot.yes_ask + snapshot.no_ask - 1.0).abs() < 0.10,
                time_remaining_ok: (snapshot.market_end_ts - ts as i64) > 45,
                bankroll_ok: true, // TODO
                cooldown_ok: true, // TODO
                daily_loss_ok: true, // TODO
                direction_unlocked: true, // TODO
                feed_stale: !execution.feed_monitor().entries_allowed(&snapshot.asset, snapshot.market_end_ts - ts as i64),
            },
        };
        
        // 4. Get strategy decision
        let decision = strategy.on_observation(&obs);
        
        // 5. Execute decision
        if let Some(result) = execution.on_decision(&decision, &obs).await {
            log_execution_result(&result, ts);
        }
        
        // 6. Check timeouts
        let timeouts = execution.position_tracker_mut().check_timeouts(ts);
        for timeout in timeouts {
            log_timeout(&timeout, ts);
        }
        
        // 7. Check for market end
        if ts >= snapshot.market_end_ts as u64 {
            println!("[MARKET_END] market={}", snapshot.market_slug);
            strategy.reset();
        }
        
        // 8. Check shutdown signal
        if config.shutdown_requested() {
            break;
        }
        
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    Ok(())
}

fn log_execution_result(result: &ExecutionResult, ts: u64) {
    match result {
        ExecutionResult::Entered { position, fill } => {
            println!(
                "[ENTRY] ts={} dir={:?} price={:.4} size={:.2f} order={}",
                ts, position.direction, fill.filled_price, fill.filled_size_usd, fill.order_id
            );
        }
        ExecutionResult::Exited { pnl, reason, .. } => {
            println!("[EXIT] ts={} pnl={:.4} reason={:?}", ts, pnl, reason);
        }
        ExecutionResult::Blocked { reason } => {
            // Already logged in engine
        }
        ExecutionResult::NoFill { request } => {
            println!("[NO_FILL] ts={} market={}", ts, request.market_slug);
        }
        ExecutionResult::DryRun { request } => {
            println!("[DRY_RUN] ts={} market={} dir={:?}", ts, request.market_slug, request.direction);
        }
        ExecutionResult::Error { request, error } => {
            eprintln!("[ERROR] ts={} market={} error={}", ts, request.market_slug, error);
        }
    }
}

fn log_timeout(timeout: &TimeoutResult, ts: u64) {
    match timeout {
        TimeoutResult::EntryTimeout { order_id } => {
            eprintln!("[TIMEOUT] ts={} type=entry order={}", ts, order_id);
        }
        TimeoutResult::ExitTimeout { order_id } => {
            eprintln!("[TIMEOUT] ts={} type=exit order={}", ts, order_id);
        }
    }
}
```

## 4. CLI Integration

### 4.1 New flags

```bash
# Watch market with FAK execution (shadow mode)
polymarket bot watch-btc \
  --engine fused \
  --execution shadow \
  --entry-type FAK

# Live trade with FAK and fill confirmation
polymarket bot trade-btc \
  --engine fused \
  --execution live \
  --entry-type FAK \
  --require-fill-confirmation \
  --fill-timeout 5s

# Dry run to see what would happen
polymarket bot trade-btc \
  --engine fused \
  --execution dry-run

# Set stale feed thresholds
polymarket bot watch-btc \
  --max-stale-healthy 5s \
  --max-stale-tradeable 30s \
  --short-market-stale 2s
```

### 4.2 Configuration file

```yaml
# execution.yaml
execution:
  mode: shadow  # shadow, dry-run, live
  
  entry_order_type: FAK
  exit_order_type: FAK
  
  max_feed_stale_s: 5
  short_market_threshold_s: 300
  
  require_fill_confirmation: true
  fill_confirmation_timeout_s: 5
  
  slippage_tolerance: 0.02
  min_fill_usd: 1.0
  
  fok_fallback: true

feed_monitor:
  max_stale_healthy_s: 5
  max_stale_tradeable_s: 30
  short_market_max_stale_s: 2
```

## 5. Test Plan

### 5.1 Unit tests

- `FAK` orders accept partial fills above min threshold
- `FOK` orders reject partial fills
- Fill confirmation updates position tracker
- Stale feed blocks entries but allows exits
- Timeout detection works correctly

### 5.2 Integration tests

- Shadow execution produces same results as current `shadow.rs`
- Dry-run logs orders without submitting
- Live execution (testnet) submits and confirms fills
- Position tracker correctly records entry/exit pairs

### 5.3 Acceptance criteria

- [ ] FAK entries accept partial fills >= min threshold
- [ ] FOK entries fail completely if full fill not available
- [ ] Fill confirmation updates position before next entry
- [ ] Stale feed (age > max_stale_healthy_s) blocks new entries
- [ ] Stale feed (age <= max_stale_tradeable_s) allows exits
- [ ] Short markets (<= 5m) require feed age < 2s
- [ ] Timeouts cancel pending orders after configured duration
- [ ] Trade history correctly records all entry/exit pairs
- [ ] Replay produces identical results to live shadow mode

## 6. Migration Path

### Phase 1: Extract types

1. Create `src/bot/execution/types.rs` with all types
2. Create `src/bot/execution/mod.rs` with trait definitions
3. No behavior changes

### Phase 2: Implement execution engine

1. Implement `ExecutionEngine` using existing `execution.rs` logic
2. Add `FAK` support via CLOB SDK
3. Wire to CLI with `--execution` flag

### Phase 3: Add fill confirmation

1. Implement `PositionTracker` with pending state
2. Add timeout detection
3. Add fill confirmation requirement

### Phase 4: Add stale-feed handling

1. Implement `FeedMonitor`
2. Add staleness checks to execution engine
3. Add exit-only mode for stale feeds

### Phase 5: Deprecate old code

1. Mark `shadow.rs` as deprecated
2. Update all callers to use `ExecutionEngine`
3. Remove old code after validation

## 7. References

- Polymarket CLOB API: https://docs.polymarket.com/clob-api
- Order types: https://docs.polymarket.com/clob-api/orders
- Fee structure: https://docs.polymarket.com/fees
- Fill confirmation: https://docs.polymarket.com/clob-api/websocket
