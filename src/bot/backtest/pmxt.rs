use anyhow::{Context, Result};
use duckdb::Connection;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::data::Trade;

const HTTPFS_SETUP: &str = r#"
INSTALL httpfs;
LOAD httpfs;
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PmxtRow {
    pub market_slug: String,
    pub timestamp: i64,
    pub yes_midpoint: f64,
    pub no_midpoint: f64,
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub volume: f64,
}

pub struct PmxtFetcher {
    conn: Connection,
    filter_pattern: String,
}

impl PmxtFetcher {
    pub fn new() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("Failed to create DuckDB in-memory connection")?;

        conn.execute_batch(HTTPFS_SETUP)
            .context("Failed to install/load httpfs extension")?;

        Ok(Self {
            conn,
            filter_pattern: "btc-updown-5m".to_string(),
        })
    }

    pub fn with_filter(mut self, pattern: &str) -> Self {
        self.filter_pattern = pattern.to_lowercase();
        self
    }

    pub fn fetch_url(&self, url: &str) -> Result<Vec<PmxtRow>> {
        println!("[PMXT] Querying remote Parquet: {}", url);
        println!("[PMXT] Filter pattern: {}%", self.filter_pattern);

        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {msg}")
                .unwrap(),
        );
        bar.set_message("Fetching data via HTTP range requests...");
        bar.enable_steady_tick(std::time::Duration::from_millis(100));

        let sql = format!(
            r#"
SELECT
    market_slug,
    timestamp,
    COALESCE(yes_midpoint, 0.0) AS yes_midpoint,
    COALESCE(no_midpoint, 0.0) AS no_midpoint,
    COALESCE(yes_bid, 0.0) AS yes_bid,
    COALESCE(yes_ask, 0.0) AS yes_ask,
    COALESCE(no_bid, 0.0) AS no_bid,
    COALESCE(no_ask, 0.0) AS no_ask,
    COALESCE(volume, 0.0) AS volume
FROM read_parquet('{}')
WHERE lower(market_slug) LIKE '%{}%'
"#,
            url, self.filter_pattern
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("Failed to prepare SQL query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(PmxtRow {
                    market_slug: row.get(0)?,
                    timestamp: row.get(1)?,
                    yes_midpoint: row.get(2)?,
                    no_midpoint: row.get(3)?,
                    yes_bid: row.get(4)?,
                    yes_ask: row.get(5)?,
                    no_bid: row.get(6)?,
                    no_ask: row.get(7)?,
                    volume: row.get(8)?,
                })
            })
            .context("Failed to execute query")?;

        let mut results = Vec::new();
        for row in rows {
            match row {
                Ok(r) => results.push(r),
                Err(e) => {
                    bar.finish_and_clear();
                    eprintln!("[PMXT] Warning: Failed to parse row: {}", e);
                }
            }
        }

        bar.finish_with_message(format!("Fetched {} rows", results.len()));

        Ok(results)
    }

    pub fn to_trades(rows: &[PmxtRow]) -> Vec<Trade> {
        rows.iter()
            .filter(|r| r.yes_midpoint > 0.0 && r.yes_midpoint < 1.0)
            .map(|r| Trade {
                market_slug: r.market_slug.clone(),
                token_id: String::new(),
                timestamp: r.timestamp,
                price: r.yes_midpoint,
                size: r.volume,
                side: "BUY".to_string(),
                resolved_outcome: None,
            })
            .collect()
    }

    pub fn export_csv<P: AsRef<Path>>(rows: &[PmxtRow], path: P) -> Result<usize> {
        let path = path.as_ref();
        println!(
            "[PMXT] Exporting {} rows to CSV: {}",
            rows.len(),
            path.display()
        );

        let mut wtr = csv::Writer::from_path(path).context("Failed to create CSV writer")?;

        for row in rows {
            wtr.serialize(row).with_context(|| {
                format!("Failed to serialize row for market: {}", row.market_slug)
            })?;
        }

        wtr.flush().context("Failed to flush CSV file")?;
        println!("[PMXT] Export complete: {} rows written", rows.len());

        Ok(rows.len())
    }

    pub fn export_parquet_direct(&self, url: &str, output_path: &str) -> Result<()> {
        println!("[PMXT] Direct export from {} to {}", url, output_path);

        let sql = format!(
            r#"
COPY (
    SELECT
        market_slug,
        timestamp,
        COALESCE(yes_midpoint, 0.0) AS yes_midpoint,
        COALESCE(no_midpoint, 0.0) AS no_midpoint,
        COALESCE(yes_bid, 0.0) AS yes_bid,
        COALESCE(yes_ask, 0.0) AS yes_ask,
        COALESCE(no_bid, 0.0) AS no_bid,
        COALESCE(no_ask, 0.0) AS no_ask,
        COALESCE(volume, 0.0) AS volume
    FROM read_parquet('{}')
    WHERE lower(market_slug) LIKE '%{}%'
) TO '{}' (FORMAT PARQUET);
"#,
            url, self.filter_pattern, output_path
        );

        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {msg}")
                .unwrap(),
        );
        bar.set_message("Exporting filtered data directly...");
        bar.enable_steady_tick(std::time::Duration::from_millis(100));

        self.conn
            .execute_batch(&sql)
            .context("Failed to export to Parquet")?;

        bar.finish_with_message("Export complete".to_string());
        println!("[PMXT] Filtered data exported to: {}", output_path);

        Ok(())
    }
}

impl Default for PmxtFetcher {
    fn default() -> Self {
        Self::new().expect("Failed to create default PmxtFetcher")
    }
}

pub fn fetch_btc_updown(url: &str) -> Result<Vec<PmxtRow>> {
    let fetcher = PmxtFetcher::new()?;
    fetcher.fetch_url(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetcher_creation() {
        let fetcher = PmxtFetcher::new();
        assert!(fetcher.is_ok());
    }
}
