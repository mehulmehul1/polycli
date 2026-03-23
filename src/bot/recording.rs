use chrono::Utc;
use std::fs::{self, File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// Records live orderbook ticks to CSV files for later backtesting.
/// One file per market: recordings/btc-updown-5m-1772784000.csv
/// Or one session file: recordings/session_20260322_210000.csv
pub struct TickRecorder {
    session_dir: PathBuf,
    session_file: Option<BufWriter<File>>,
    csv_writer: Option<csv::Writer<BufWriter<File>>>,
    current_market: String,
    tick_count: usize,
    session_tick_count: usize,
}

#[derive(serde::Serialize)]
struct TickRecord {
    timestamp: f64,
    market_slug: String,
    yes_bid: f64,
    yes_ask: f64,
    no_bid: f64,
    no_ask: f64,
    time_remaining: i64,
}

impl TickRecorder {
    /// Create a new recorder. Session file goes in recordings/session_YYYYMMDD_HHMMSS.csv
    pub fn new(recordings_dir: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = Path::new(recordings_dir);
        fs::create_dir_all(dir)?;

        let now = Utc::now().format("%Y%m%d_%H%M%S");
        let session_path = dir.join(format!("session_{}.csv", now));

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&session_path)?;

        let mut writer = csv::Writer::from_writer(BufWriter::new(file));

        // Write header
        writer.write_record([
            "timestamp",
            "market_slug",
            "yes_bid",
            "yes_ask",
            "no_bid",
            "no_ask",
            "time_remaining",
        ])?;
        writer.flush()?;

        println!("[RECORDER] Recording to: {}", session_path.display());

        Ok(Self {
            session_dir: dir.to_path_buf(),
            session_file: None,
            csv_writer: Some(writer),
            current_market: String::new(),
            tick_count: 0,
            session_tick_count: 0,
        })
    }

    /// Record a single tick. Call this for every snapshot in the live loop.
    pub fn record_tick(
        &mut self,
        timestamp: f64,
        market_slug: &str,
        yes_bid: f64,
        yes_ask: f64,
        no_bid: f64,
        no_ask: f64,
        time_remaining: i64,
    ) {
        // Track market transitions
        if market_slug != self.current_market {
            if !self.current_market.is_empty() {
                println!(
                    "[RECORDER] Market {} finished: {} ticks recorded",
                    self.current_market, self.tick_count
                );
            }
            self.current_market = market_slug.to_string();
            self.tick_count = 0;
        }

        if let Some(ref mut writer) = self.csv_writer {
            let _ = writer.write_record(&[
                format!("{:.6}", timestamp),
                market_slug.to_string(),
                format!("{:.6}", yes_bid),
                format!("{:.6}", yes_ask),
                format!("{:.6}", no_bid),
                format!("{:.6}", no_ask),
                format!("{}", time_remaining),
            ]);
            self.tick_count += 1;
            self.session_tick_count += 1;

            // Flush every 1000 ticks for safety
            if self.session_tick_count % 1000 == 0 {
                let _ = writer.flush();
            }
        }
    }

    /// Flush remaining data and close
    pub fn flush(&mut self) {
        if let Some(ref mut writer) = self.csv_writer {
            let _ = writer.flush();
        }
        if !self.current_market.is_empty() {
            println!(
                "[RECORDER] Market {} finished: {} ticks recorded",
                self.current_market, self.tick_count
            );
        }
        println!(
            "[RECORDER] Session complete: {} total ticks recorded",
            self.session_tick_count
        );
    }

    /// Get total ticks recorded this session
    pub fn total_ticks(&self) -> usize {
        self.session_tick_count
    }
}

impl Drop for TickRecorder {
    fn drop(&mut self) {
        self.flush();
    }
}
