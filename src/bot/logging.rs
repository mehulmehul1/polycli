use serde::Serialize;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    BookUpdate {
        ts: u64,
        market_slug: String,
        source: String,
        yes_bid: f64,
        yes_ask: f64,
        no_bid: f64,
        no_ask: f64,
    },
    StrategySignal {
        ts: u64,
        market_slug: String,
        midpoint: f64,
        entry: String,
        exit: String,
        detail: String,
    },
    GateDecision {
        ts: u64,
        market_slug: String,
        decision: String,
        reason: String,
    },
    ShadowEntry {
        ts: u64,
        market_slug: String,
        side: String,
        price: f64,
        size_usd: f64,
        bankroll_after: f64,
    },
    ShadowExit {
        ts: u64,
        market_slug: String,
        side: String,
        price: f64,
        pnl_usd: f64,
        bankroll_after: f64,
    },
    LiveEntry {
        ts: u64,
        market_slug: String,
        side: String,
        price: f64,
        size_usd: f64,
        order_id: Option<String>,
    },
    LiveExit {
        ts: u64,
        market_slug: String,
        side: String,
        price: f64,
        pnl_usd: f64,
        order_id: Option<String>,
    },
    PendingSettlement {
        ts: u64,
        market_slug: String,
        side: String,
        bid_price: f64,
        shares: f64,
    },
    EmergencyHalt {
        ts: u64,
        market_slug: String,
        daily_pnl: f64,
        reason: String,
    },
}

#[derive(Clone)]
pub struct JsonlEventLogger {
    writer: Arc<Mutex<BufWriter<File>>>,
    path: PathBuf,
}

impl JsonlEventLogger {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log<T: Serialize>(&self, kind: &str, payload: &T) {
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "kind": kind,
            "payload": payload,
        });

        if let Ok(mut writer) = self.writer.lock() {
            let _ = serde_json::to_writer(&mut *writer, &line);
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }

    pub fn log_event(&self, payload: &EngineEvent) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = serde_json::to_writer(&mut *writer, payload);
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }
}

#[derive(Clone)]
pub struct EngineEventLoggers {
    market: JsonlEventLogger,
    strategy: JsonlEventLogger,
    execution: JsonlEventLogger,
}

impl EngineEventLoggers {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let (market_path, strategy_path, execution_path) = split_log_paths(path.as_ref());
        Ok(Self {
            market: JsonlEventLogger::new(market_path)?,
            strategy: JsonlEventLogger::new(strategy_path)?,
            execution: JsonlEventLogger::new(execution_path)?,
        })
    }

    pub fn log_market(&self, event: EngineEvent) {
        self.market.log_event(&event);
    }

    pub fn log_strategy(&self, event: EngineEvent) {
        self.strategy.log_event(&event);
    }

    pub fn log_execution(&self, event: EngineEvent) {
        self.execution.log_event(&event);
    }
}

fn split_log_paths(path: &Path) -> (PathBuf, PathBuf, PathBuf) {
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("events");
        return (
            parent.join(format!("{stem}_market.jsonl")),
            parent.join(format!("{stem}_strategy.jsonl")),
            parent.join(format!("{stem}_execution.jsonl")),
        );
    }

    (
        path.join("market_events.jsonl"),
        path.join("strategy_events.jsonl"),
        path.join("execution_events.jsonl"),
    )
}
