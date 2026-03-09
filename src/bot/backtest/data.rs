use anyhow::{Context, Result};
use arrow::array::{Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub market_slug: String,
    pub token_id: String,
    pub timestamp: i64,
    pub price: f64,
    pub size: f64,
    pub side: String,
    pub resolved_outcome: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketData {
    pub slug: String,
    pub yes_token_id: String,
    pub no_token_id: String,
    pub start_time: i64,
    pub end_time: i64,
    pub resolution: Option<bool>,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub timestamp: i64,
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub yes_midpoint: f64,
    pub volume: f64,
}

pub struct BeckerParser {
    trades: Vec<Trade>,
    markets: HashMap<String, MarketData>,
}

impl BeckerParser {
    pub fn new() -> Self {
        Self {
            trades: Vec::new(),
            markets: HashMap::new(),
        }
    }

    pub fn load_parquet<P: AsRef<Path>>(&mut self, path: P) -> Result<usize> {
        let file = File::open(&path)
            .with_context(|| format!("Failed to open parquet file: {:?}", path.as_ref()))?;

        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .context("Failed to create parquet reader")?;

        let reader = builder.build()?;

        let mut count = 0;
        for batch in reader {
            let batch = batch.context("Failed to read batch")?;
            count += self.parse_batch(&batch)?;
        }

        Ok(count)
    }

    pub fn load_csv<P: AsRef<Path>>(&mut self, path: P) -> Result<usize> {
        let file = File::open(&path)
            .with_context(|| format!("Failed to open CSV file: {:?}", path.as_ref()))?;

        let mut rdr = csv::Reader::from_reader(file);
        let mut count = 0;

        for result in rdr.deserialize() {
            let trade: Trade = result.context("Failed to deserialize CSV row")?;
            self.trades.push(trade);
            count += 1;
        }

        Ok(count)
    }

    fn parse_batch(&mut self, batch: &RecordBatch) -> Result<usize> {
        let schema = batch.schema();
        let columns: Vec<_> = schema.fields().iter().map(|f| f.name().as_str()).collect();

        let get_col = |name: &str| -> Option<usize> { columns.iter().position(|&c| c == name) };

        let market_col = get_col("market_slug")
            .or_else(|| get_col("slug"))
            .or_else(|| get_col("market"));
        let token_col = get_col("token_id").or_else(|| get_col("token"));
        let ts_col = get_col("timestamp")
            .or_else(|| get_col("ts"))
            .or_else(|| get_col("time"));
        let price_col = get_col("price").or_else(|| get_col("trade_price"));
        let size_col = get_col("size")
            .or_else(|| get_col("amount"))
            .or_else(|| get_col("volume"));
        let side_col = get_col("side").or_else(|| get_col("direction"));

        let num_rows = batch.num_rows();

        let markets: Option<&StringArray> =
            market_col.and_then(|i| batch.column(i).as_any().downcast_ref());
        let tokens: Option<&StringArray> =
            token_col.and_then(|i| batch.column(i).as_any().downcast_ref());
        let timestamps: Option<&Int64Array> =
            ts_col.and_then(|i| batch.column(i).as_any().downcast_ref());
        let prices: Option<&Float64Array> =
            price_col.and_then(|i| batch.column(i).as_any().downcast_ref());
        let sizes: Option<&Float64Array> =
            size_col.and_then(|i| batch.column(i).as_any().downcast_ref());
        let sides: Option<&StringArray> =
            side_col.and_then(|i| batch.column(i).as_any().downcast_ref());

        let mut count = 0;
        for i in 0..num_rows {
            let market_slug = markets.map(|m| m.value(i).to_string()).unwrap_or_default();
            let token_id = tokens.map(|t| t.value(i).to_string()).unwrap_or_default();
            let timestamp = timestamps.map(|t| t.value(i)).unwrap_or(0);
            let price = prices.map(|p| p.value(i)).unwrap_or(0.0);
            let size = sizes.map(|s| s.value(i)).unwrap_or(0.0);
            let side = sides
                .map(|s| s.value(i).to_string())
                .unwrap_or_else(|| "BUY".to_string());

            if price > 0.0 && price < 1.0 && !market_slug.is_empty() {
                self.trades.push(Trade {
                    market_slug,
                    token_id,
                    timestamp,
                    price,
                    size,
                    side,
                    resolved_outcome: None,
                });
                count += 1;
            }
        }

        Ok(count)
    }

    pub fn organize_by_market(&mut self) -> &HashMap<String, MarketData> {
        self.trades.sort_by_key(|t| t.timestamp);

        let mut by_market: HashMap<String, Vec<&Trade>> = HashMap::new();
        for trade in &self.trades {
            by_market
                .entry(trade.market_slug.clone())
                .or_default()
                .push(trade);
        }

        for (slug, trades) in by_market {
            if trades.is_empty() {
                continue;
            }

            let timestamps: Vec<i64> = trades.iter().map(|t| t.timestamp).collect();
            let start_time = *timestamps.iter().min().unwrap_or(&0);
            let end_time = *timestamps.iter().max().unwrap_or(&0);

            let first_trade = trades.first().unwrap();

            let market_data = MarketData {
                slug: slug.clone(),
                yes_token_id: first_trade.token_id.clone(),
                no_token_id: String::new(),
                start_time,
                end_time,
                resolution: None,
                trades: trades.iter().map(|t| (*t).clone()).collect(),
            };

            self.markets.insert(slug, market_data);
        }

        &self.markets
    }

    pub fn get_markets(&self) -> &HashMap<String, MarketData> {
        &self.markets
    }

    pub fn get_trades(&self) -> &[Trade] {
        &self.trades
    }

    pub fn generate_snapshots(
        &self,
        market_slug: &str,
        interval_seconds: i64,
    ) -> Result<Vec<PriceSnapshot>> {
        let market = self
            .markets
            .get(market_slug)
            .with_context(|| format!("Market not found: {}", market_slug))?;

        let trades = &market.trades;
        if trades.is_empty() {
            return Ok(Vec::new());
        }

        let start = trades.first().unwrap().timestamp;
        let end = trades.last().unwrap().timestamp;

        let mut snapshots = Vec::new();
        let mut trade_iter = trades.iter().peekable();

        for bucket_start in (start..=end).step_by(interval_seconds as usize) {
            let bucket_end = bucket_start + interval_seconds;

            let mut bucket_trades: Vec<&Trade> = Vec::new();

            while let Some(trade) = trade_iter.peek() {
                if trade.timestamp < bucket_end {
                    bucket_trades.push(trade_iter.next().unwrap());
                } else {
                    break;
                }
            }

            if bucket_trades.is_empty() {
                continue;
            }

            let last_trade = bucket_trades.last().unwrap();
            let price = last_trade.price;
            let volume: f64 = bucket_trades.iter().map(|t| t.size).sum();

            snapshots.push(PriceSnapshot {
                timestamp: bucket_start,
                yes_bid: (price - 0.01).max(0.01),
                yes_ask: (price + 0.01).min(0.99),
                no_bid: (1.0 - price - 0.01).max(0.01),
                no_ask: (1.0 - price + 0.01).min(0.99),
                yes_midpoint: price,
                volume,
            });
        }

        Ok(snapshots)
    }
}

impl Default for BeckerParser {
    fn default() -> Self {
        Self::new()
    }
}

pub fn load_mock_data() -> Vec<MarketData> {
    let mut markets = Vec::new();

    for market_idx in 0..10 {
        let slug = format!("btc-updown-5m-{}", 1700000000 + market_idx * 300);
        let base_time = 1700000000 + market_idx * 300;

        let mut trades = Vec::new();
        let mut price = 0.50;

        for i in 0..60 {
            let delta = if i % 3 == 0 {
                0.005_f64
            } else if i % 3 == 1 {
                -0.003_f64
            } else {
                0.001_f64
            };
            price = (price + delta).clamp(0.35_f64, 0.65_f64);

            trades.push(Trade {
                market_slug: slug.clone(),
                token_id: format!("token_yes_{}", market_idx),
                timestamp: base_time + i as i64,
                price,
                size: 100.0 + (i as f64 * 10.0),
                side: if i % 2 == 0 {
                    "BUY".to_string()
                } else {
                    "SELL".to_string()
                },
                resolved_outcome: Some(price > 0.50),
            });
        }

        markets.push(MarketData {
            slug: slug.clone(),
            yes_token_id: format!("token_yes_{}", market_idx),
            no_token_id: format!("token_no_{}", market_idx),
            start_time: base_time,
            end_time: base_time + 300,
            resolution: Some(trades.last().map(|t| t.price > 0.50).unwrap_or(false)),
            trades,
        });
    }

    markets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_data_generation() {
        let markets = load_mock_data();
        assert_eq!(markets.len(), 10);

        let first = &markets[0];
        assert!(first.slug.starts_with("btc-updown-5m-"));
        assert_eq!(first.trades.len(), 60);
    }
}
