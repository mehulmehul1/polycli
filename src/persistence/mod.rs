pub mod sqlite;

use anyhow::Result;
use polymarket_client_sdk::types::U256;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenSide {
    Yes,
    No,
}

impl std::fmt::Display for TokenSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenSide::Yes => write!(f, "YES"),
            TokenSide::No => write!(f, "NO"),
        }
    }
}

impl From<&str> for TokenSide {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "YES" | "LONG" => TokenSide::Yes,
            _ => TokenSide::No,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeLog {
    pub id: Option<i64>,
    pub market_slug: String,
    pub token_side: TokenSide,
    pub entry_price: f64,
    pub exit_price: f64,
    pub size_usd: f64,
    pub pnl_usd: f64,
    pub timestamp_entry: i64,
    pub timestamp_exit: i64,
    pub order_id_entry: String,
    pub order_id_exit: Option<String>,
    pub slippage_entry_bps: i64,
    pub slippage_exit_bps: i64,
    pub latency_entry_ms: i64,
    pub latency_exit_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionState {
    pub token_id: U256,
    pub market_slug: String,
    pub token_side: TokenSide,
    pub size: f64,
    pub entry_price: f64,
    pub size_usd: f64,
    pub entry_order_id: String,
    pub entry_timestamp: i64,
    pub entry_slippage_bps: i64,
    pub entry_latency_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotState {
    pub bankroll: f64,
    pub starting_capital: f64,
    pub peak_bankroll: f64,
    pub daily_pnl: f64,
    pub daily_start_timestamp: i64,
    pub consecutive_losses: usize,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub total_pnl: f64,
    pub killed: bool,
    pub kill_reason: Option<String>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            bankroll: 4.0,
            starting_capital: 4.0,
            peak_bankroll: 4.0,
            daily_pnl: 0.0,
            daily_start_timestamp: 0,
            consecutive_losses: 0,
            total_trades: 0,
            winning_trades: 0,
            total_pnl: 0.0,
            killed: false,
            kill_reason: None,
        }
    }
}

pub trait StateStore: Send + Sync {
    fn save_position(&self, pos: &PositionState) -> Result<()>;
    fn load_active_position(&self) -> Result<Option<PositionState>>;
    fn clear_position(&self) -> Result<()>;
    
    fn save_trade(&self, trade: &TradeLog) -> Result<i64>;
    fn get_trades(&self, limit: Option<usize>) -> Result<Vec<TradeLog>>;
    fn get_trade_count(&self) -> Result<usize>;
    
    fn get_bot_state(&self) -> Result<BotState>;
    fn update_bot_state(&self, state: &BotState) -> Result<()>;
    
    fn get_bankroll(&self) -> Result<f64>;
    fn update_bankroll(&self, new_balance: f64) -> Result<()>;
    
    fn record_daily_reset(&self) -> Result<()>;
    fn check_and_reset_daily(&self, current_timestamp: i64) -> Result<bool>;
}

pub fn generate_order_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
