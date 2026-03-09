use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

use super::{BotState, PositionState, StateStore, TokenSide, TradeLog};

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize_schema()?;
        store.initialize_bot_state_if_missing()?;

        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("Failed to create in-memory SQLite database")?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize_schema()?;
        store.initialize_bot_state_if_missing()?;

        Ok(store)
    }

    fn initialize_schema(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS positions (
                token_id TEXT PRIMARY KEY,
                market_slug TEXT NOT NULL,
                token_side TEXT NOT NULL,
                size REAL NOT NULL,
                entry_price REAL NOT NULL,
                size_usd REAL NOT NULL,
                entry_order_id TEXT NOT NULL,
                entry_timestamp INTEGER NOT NULL,
                entry_slippage_bps INTEGER NOT NULL,
                entry_latency_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                market_slug TEXT NOT NULL,
                token_side TEXT NOT NULL,
                entry_price REAL NOT NULL,
                exit_price REAL NOT NULL,
                size_usd REAL NOT NULL,
                pnl_usd REAL NOT NULL,
                timestamp_entry INTEGER NOT NULL,
                timestamp_exit INTEGER NOT NULL,
                order_id_entry TEXT NOT NULL,
                order_id_exit TEXT,
                slippage_entry_bps INTEGER NOT NULL,
                slippage_exit_bps INTEGER NOT NULL,
                latency_entry_ms INTEGER NOT NULL,
                latency_exit_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS bot_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                bankroll REAL NOT NULL,
                starting_capital REAL NOT NULL,
                peak_bankroll REAL NOT NULL,
                daily_pnl REAL NOT NULL,
                daily_start_timestamp INTEGER NOT NULL,
                consecutive_losses INTEGER NOT NULL,
                total_trades INTEGER NOT NULL,
                winning_trades INTEGER NOT NULL,
                total_pnl REAL NOT NULL,
                killed INTEGER NOT NULL,
                kill_reason TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_trades_timestamp ON trades(timestamp_exit);
            CREATE INDEX IF NOT EXISTS idx_trades_market ON trades(market_slug);
            "#,
        )
        .context("Failed to initialize database schema")?;

        Ok(())
    }

    fn initialize_bot_state_if_missing(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM bot_state WHERE id = 1)",
                [],
                |row| row.get(0),
            )
            .context("Failed to check if bot_state exists")?;

        if !exists {
            conn.execute(
                r#"
                INSERT INTO bot_state (
                    id, bankroll, starting_capital, peak_bankroll,
                    daily_pnl, daily_start_timestamp, consecutive_losses,
                    total_trades, winning_trades, total_pnl, killed, kill_reason
                ) VALUES (1, 4.0, 4.0, 4.0, 0.0, 0, 0, 0, 0, 0.0, 0, NULL)
                "#,
                [],
            )
            .context("Failed to initialize bot_state")?;
        }

        Ok(())
    }

    fn parse_token_id(s: &str) -> Result<polymarket_client_sdk::types::U256> {
        s.parse()
            .map_err(|e| anyhow::anyhow!("Invalid token ID '{}': {}", s, e))
    }

    fn format_token_id(id: polymarket_client_sdk::types::U256) -> String {
        id.to_string()
    }
}

impl StateStore for SqliteStore {
    fn save_position(&self, pos: &PositionState) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute(
            r#"
            INSERT OR REPLACE INTO positions (
                token_id, market_slug, token_side, size, entry_price,
                size_usd, entry_order_id, entry_timestamp,
                entry_slippage_bps, entry_latency_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                Self::format_token_id(pos.token_id),
                pos.market_slug,
                pos.token_side.to_string(),
                pos.size,
                pos.entry_price,
                pos.size_usd,
                pos.entry_order_id,
                pos.entry_timestamp,
                pos.entry_slippage_bps,
                pos.entry_latency_ms,
            ],
        )
        .context("Failed to save position")?;

        Ok(())
    }

    fn load_active_position(&self) -> Result<Option<PositionState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT token_id, market_slug, token_side, size, entry_price,
                       size_usd, entry_order_id, entry_timestamp,
                       entry_slippage_bps, entry_latency_ms
                FROM positions
                LIMIT 1
                "#,
            )
            .context("Failed to prepare position query")?;

        let result = stmt
            .query_row([], |row| {
                let token_id_str: String = row.get(0)?;
                let token_id = Self::parse_token_id(&token_id_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;

                Ok(PositionState {
                    token_id,
                    market_slug: row.get(1)?,
                    token_side: TokenSide::from(row.get::<_, String>(2)?.as_str()),
                    size: row.get(3)?,
                    entry_price: row.get(4)?,
                    size_usd: row.get(5)?,
                    entry_order_id: row.get(6)?,
                    entry_timestamp: row.get(7)?,
                    entry_slippage_bps: row.get(8)?,
                    entry_latency_ms: row.get(9)?,
                })
            })
            .optional()
            .context("Failed to load position")?;

        Ok(result)
    }

    fn clear_position(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute("DELETE FROM positions", [])
            .context("Failed to clear position")?;

        Ok(())
    }

    fn save_trade(&self, trade: &TradeLog) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute(
            r#"
            INSERT INTO trades (
                market_slug, token_side, entry_price, exit_price, size_usd,
                pnl_usd, timestamp_entry, timestamp_exit, order_id_entry,
                order_id_exit, slippage_entry_bps, slippage_exit_bps,
                latency_entry_ms, latency_exit_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                trade.market_slug,
                trade.token_side.to_string(),
                trade.entry_price,
                trade.exit_price,
                trade.size_usd,
                trade.pnl_usd,
                trade.timestamp_entry,
                trade.timestamp_exit,
                trade.order_id_entry,
                trade.order_id_exit,
                trade.slippage_entry_bps,
                trade.slippage_exit_bps,
                trade.latency_entry_ms,
                trade.latency_exit_ms,
            ],
        )
        .context("Failed to save trade")?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    fn get_trades(&self, limit: Option<usize>) -> Result<Vec<TradeLog>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let sql = match limit {
            Some(n) => format!(
                "SELECT id, market_slug, token_side, entry_price, exit_price, size_usd, \
                 pnl_usd, timestamp_entry, timestamp_exit, order_id_entry, order_id_exit, \
                 slippage_entry_bps, slippage_exit_bps, latency_entry_ms, latency_exit_ms \
                 FROM trades ORDER BY timestamp_exit DESC LIMIT {}",
                n
            ),
            None => "SELECT id, market_slug, token_side, entry_price, exit_price, size_usd, \
                     pnl_usd, timestamp_entry, timestamp_exit, order_id_entry, order_id_exit, \
                     slippage_entry_bps, slippage_exit_bps, latency_entry_ms, latency_exit_ms \
                     FROM trades ORDER BY timestamp_exit DESC"
                .to_string(),
        };

        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare trades query")?;

        let trades = stmt
            .query_map([], |row| {
                Ok(TradeLog {
                    id: Some(row.get(0)?),
                    market_slug: row.get(1)?,
                    token_side: TokenSide::from(row.get::<_, String>(2)?.as_str()),
                    entry_price: row.get(3)?,
                    exit_price: row.get(4)?,
                    size_usd: row.get(5)?,
                    pnl_usd: row.get(6)?,
                    timestamp_entry: row.get(7)?,
                    timestamp_exit: row.get(8)?,
                    order_id_entry: row.get(9)?,
                    order_id_exit: row.get(10)?,
                    slippage_entry_bps: row.get(11)?,
                    slippage_exit_bps: row.get(12)?,
                    latency_entry_ms: row.get(13)?,
                    latency_exit_ms: row.get(14)?,
                })
            })
            .context("Failed to query trades")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect trades")?;

        Ok(trades)
    }

    fn get_trade_count(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))
            .context("Failed to count trades")?;

        Ok(count)
    }

    fn get_bot_state(&self) -> Result<BotState> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let state = conn
            .query_row(
                r#"
                SELECT bankroll, starting_capital, peak_bankroll, daily_pnl,
                       daily_start_timestamp, consecutive_losses, total_trades,
                       winning_trades, total_pnl, killed, kill_reason
                FROM bot_state
                WHERE id = 1
                "#,
                [],
                |row| {
                    Ok(BotState {
                        bankroll: row.get(0)?,
                        starting_capital: row.get(1)?,
                        peak_bankroll: row.get(2)?,
                        daily_pnl: row.get(3)?,
                        daily_start_timestamp: row.get(4)?,
                        consecutive_losses: row.get(5)?,
                        total_trades: row.get(6)?,
                        winning_trades: row.get(7)?,
                        total_pnl: row.get(8)?,
                        killed: row.get::<_, i64>(9)? != 0,
                        kill_reason: row.get(10)?,
                    })
                },
            )
            .context("Failed to get bot state")?;

        Ok(state)
    }

    fn update_bot_state(&self, state: &BotState) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute(
            r#"
            UPDATE bot_state SET
                bankroll = ?1,
                starting_capital = ?2,
                peak_bankroll = ?3,
                daily_pnl = ?4,
                daily_start_timestamp = ?5,
                consecutive_losses = ?6,
                total_trades = ?7,
                winning_trades = ?8,
                total_pnl = ?9,
                killed = ?10,
                kill_reason = ?11
            WHERE id = 1
            "#,
            params![
                state.bankroll,
                state.starting_capital,
                state.peak_bankroll,
                state.daily_pnl,
                state.daily_start_timestamp,
                state.consecutive_losses as i64,
                state.total_trades as i64,
                state.winning_trades as i64,
                state.total_pnl,
                if state.killed { 1i64 } else { 0i64 },
                state.kill_reason,
            ],
        )
        .context("Failed to update bot state")?;

        Ok(())
    }

    fn get_bankroll(&self) -> Result<f64> {
        let state = self.get_bot_state()?;
        Ok(state.bankroll)
    }

    fn update_bankroll(&self, new_balance: f64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute(
            "UPDATE bot_state SET bankroll = ?1, peak_bankroll = MAX(peak_bankroll, ?1) WHERE id = 1",
            params![new_balance],
        )
        .context("Failed to update bankroll")?;

        Ok(())
    }

    fn record_daily_reset(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        conn.execute(
            "UPDATE bot_state SET daily_pnl = 0.0, daily_start_timestamp = (SELECT strftime('%s', 'now')) WHERE id = 1",
            [],
        )
        .context("Failed to record daily reset")?;

        Ok(())
    }

    fn check_and_reset_daily(&self, current_timestamp: i64) -> Result<bool> {
        let state = self.get_bot_state()?;

        let current_day = current_timestamp / 86400;
        let stored_day = state.daily_start_timestamp / 86400;

        if current_day > stored_day {
            self.record_daily_reset()?;
            return Ok(true);
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_store_creation() {
        let store = SqliteStore::in_memory().expect("Failed to create store");
        let state = store.get_bot_state().expect("Failed to get state");
        assert_eq!(state.bankroll, 4.0);
        assert_eq!(state.starting_capital, 4.0);
    }

    #[test]
    fn test_save_and_load_position() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        let pos = PositionState {
            token_id: polymarket_client_sdk::types::U256::from(12345u64),
            market_slug: "btc-updown-5m-test".to_string(),
            token_side: TokenSide::Yes,
            size: 10.0,
            entry_price: 0.52,
            size_usd: 1.0,
            entry_order_id: "order-123".to_string(),
            entry_timestamp: 1700000000,
            entry_slippage_bps: 5,
            entry_latency_ms: 150,
        };

        store.save_position(&pos).expect("Failed to save position");

        let loaded = store
            .load_active_position()
            .expect("Failed to load position");
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.token_id, pos.token_id);
        assert_eq!(loaded.market_slug, pos.market_slug);
        assert_eq!(loaded.entry_price, pos.entry_price);
        assert_eq!(loaded.entry_order_id, pos.entry_order_id);
    }

    #[test]
    fn test_clear_position() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        let pos = PositionState {
            token_id: polymarket_client_sdk::types::U256::from(12345u64),
            market_slug: "test".to_string(),
            token_side: TokenSide::No,
            size: 5.0,
            entry_price: 0.45,
            size_usd: 1.0,
            entry_order_id: "order-456".to_string(),
            entry_timestamp: 1700000000,
            entry_slippage_bps: 3,
            entry_latency_ms: 200,
        };

        store.save_position(&pos).expect("Failed to save");

        let loaded = store.load_active_position().expect("Failed to load");
        assert!(loaded.is_some());

        store.clear_position().expect("Failed to clear");

        let loaded = store.load_active_position().expect("Failed to load");
        assert!(loaded.is_none());
    }

    #[test]
    fn test_save_and_get_trades() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        let trade = TradeLog {
            id: None,
            market_slug: "btc-updown-5m-test".to_string(),
            token_side: TokenSide::Yes,
            entry_price: 0.50,
            exit_price: 0.55,
            size_usd: 1.0,
            pnl_usd: 0.10,
            timestamp_entry: 1700000000,
            timestamp_exit: 1700000100,
            order_id_entry: "entry-123".to_string(),
            order_id_exit: Some("exit-456".to_string()),
            slippage_entry_bps: 5,
            slippage_exit_bps: 3,
            latency_entry_ms: 150,
            latency_exit_ms: 120,
        };

        let id = store.save_trade(&trade).expect("Failed to save trade");
        assert!(id > 0);

        let trades = store.get_trades(Some(10)).expect("Failed to get trades");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].market_slug, "btc-updown-5m-test");
        assert_eq!(trades[0].pnl_usd, 0.10);

        let count = store.get_trade_count().expect("Failed to count");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_update_bankroll() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        assert_eq!(store.get_bankroll().unwrap(), 4.0);

        store.update_bankroll(4.50).expect("Failed to update");
        assert_eq!(store.get_bankroll().unwrap(), 4.50);

        let state = store.get_bot_state().unwrap();
        assert_eq!(state.peak_bankroll, 4.50);

        store.update_bankroll(4.20).expect("Failed to update");
        let state = store.get_bot_state().unwrap();
        assert_eq!(state.bankroll, 4.20);
        assert_eq!(state.peak_bankroll, 4.50);
    }

    #[test]
    fn test_update_bot_state() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        let mut state = store.get_bot_state().unwrap();
        state.total_trades = 10;
        state.winning_trades = 7;
        state.total_pnl = 2.50;
        state.consecutive_losses = 0;
        state.killed = true;
        state.kill_reason = Some("Drawdown exceeded".to_string());

        store.update_bot_state(&state).expect("Failed to update");

        let loaded = store.get_bot_state().unwrap();
        assert_eq!(loaded.total_trades, 10);
        assert_eq!(loaded.winning_trades, 7);
        assert_eq!(loaded.total_pnl, 2.50);
        assert!(loaded.killed);
        assert_eq!(loaded.kill_reason, Some("Drawdown exceeded".to_string()));
    }

    #[test]
    fn test_daily_reset() {
        let store = SqliteStore::in_memory().expect("Failed to create store");

        let mut state = store.get_bot_state().unwrap();
        state.daily_pnl = -0.50;
        state.daily_start_timestamp = 1700000000;
        store.update_bot_state(&state).expect("Failed to update");

        let current_timestamp = 1700000000 + 86400 + 3600;
        let reset = store
            .check_and_reset_daily(current_timestamp)
            .expect("Failed to check");
        assert!(reset);

        let state = store.get_bot_state().unwrap();
        assert_eq!(state.daily_pnl, 0.0);
    }

    #[test]
    fn test_token_side_conversion() {
        assert!(matches!(TokenSide::from("YES"), TokenSide::Yes));
        assert!(matches!(TokenSide::from("yes"), TokenSide::Yes));
        assert!(matches!(TokenSide::from("LONG"), TokenSide::Yes));
        assert!(matches!(TokenSide::from("NO"), TokenSide::No));
        assert!(matches!(TokenSide::from("no"), TokenSide::No));
        assert!(matches!(TokenSide::from("SHORT"), TokenSide::No));
    }
}
