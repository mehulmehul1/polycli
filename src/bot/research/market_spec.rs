//! Market Specification
//!
//! Types for market classification and filtering.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Supported assets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SupportedAsset {
    Btc,
    Eth,
    Sol,
    Xrp,
}

impl SupportedAsset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Btc => "btc",
            Self::Eth => "eth",
            Self::Sol => "sol",
            Self::Xrp => "xrp",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        let lower = slug.to_lowercase();
        if lower.contains("btc") || lower.contains("bitcoin") {
            Some(Self::Btc)
        } else if lower.contains("eth") || lower.contains("ethereum") {
            Some(Self::Eth)
        } else if lower.contains("sol") || lower.contains("solana") {
            Some(Self::Sol)
        } else if lower.contains("xrp") || lower.contains("ripple") {
            Some(Self::Xrp)
        } else {
            None
        }
    }
}

/// Supported durations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SupportedDuration {
    M5,
    M15,
    H1,
}

impl SupportedDuration {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::H1 => "1h",
        }
    }

    pub fn seconds(&self) -> i64 {
        match self {
            Self::M5 => 300,
            Self::M15 => 900,
            Self::H1 => 3600,
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        let lower = slug.to_lowercase();
        if lower.contains("-5m") || lower.contains("5min") {
            Some(Self::M5)
        } else if lower.contains("-15m") || lower.contains("15min") {
            Some(Self::M15)
        } else if lower.contains("-1h") || lower.contains("1hour") || lower.contains("60min") {
            Some(Self::H1)
        } else {
            None
        }
    }
}

/// Market family classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketFamily {
    /// Will price close higher than it opened?
    UpdownOpenClose,
    /// Will price be above X at expiry?
    ThresholdAtExpiry,
}

impl MarketFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UpdownOpenClose => "updown_open_close",
            Self::ThresholdAtExpiry => "threshold_at_expiry",
        }
    }

    pub fn from_slug(slug: &str) -> Self {
        let lower = slug.to_lowercase();
        if lower.contains("updown") || lower.contains("up-down") || lower.contains("higher") {
            Self::UpdownOpenClose
        } else {
            Self::ThresholdAtExpiry
        }
    }
}

/// Crypto binary market specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoBinaryMarketSpec {
    pub condition_id: String,
    pub slug: String,
    pub asset: SupportedAsset,
    pub duration: SupportedDuration,
    pub family: MarketFamily,
    pub strike: Option<f64>,
    pub start_ts: i64,
    pub end_ts: i64,
}

impl CryptoBinaryMarketSpec {
    /// Parse market spec from slug and metadata
    pub fn from_slug(slug: &str, condition_id: &str, start_ts: i64, end_ts: i64) -> Option<Self> {
        let asset = SupportedAsset::from_slug(slug)?;
        let duration = SupportedDuration::from_slug(slug).unwrap_or(SupportedDuration::M5);
        let family = MarketFamily::from_slug(slug);

        // Try to extract strike from slug
        let strike = extract_strike(slug);

        Some(Self {
            condition_id: condition_id.to_string(),
            slug: slug.to_string(),
            asset,
            duration,
            family,
            strike,
            start_ts,
            end_ts,
        })
    }

    /// Check if this market is supported for trading
    pub fn is_supported(&self) -> bool {
        // Add any filtering logic here
        true
    }

    /// Get duration in seconds
    pub fn duration_seconds(&self) -> i64 {
        self.duration.seconds()
    }
}

/// Extract strike price from slug
fn extract_strike(slug: &str) -> Option<f64> {
    // Look for patterns like "85k", "100000", "above-85000"
    let lower = slug.to_lowercase();

    // Pattern: -85k, -100k
    if let Some(cap) = regex::Regex::new(r"-(\d+)k")
        .ok()?
        .captures(&lower)
    {
        if let Some(m) = cap.get(1) {
            if let Ok(k) = m.as_str().parse::<f64>() {
                return Some(k * 1000.0);
            }
        }
    }

    // Pattern: -85000, -100000
    if let Some(cap) = regex::Regex::new(r"-(\d{4,})(?:-|$)")
        .ok()?
        .captures(&lower)
    {
        if let Some(m) = cap.get(1) {
            if let Ok(v) = m.as_str().parse::<f64>() {
                return Some(v);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_btc_asset() {
        assert_eq!(
            SupportedAsset::from_slug("btc-updown-5m-123"),
            Some(SupportedAsset::Btc)
        );
    }

    #[test]
    fn parse_5m_duration() {
        assert_eq!(
            SupportedDuration::from_slug("btc-updown-5m-123"),
            Some(SupportedDuration::M5)
        );
    }

    #[test]
    fn parse_market_spec() {
        let spec = CryptoBinaryMarketSpec::from_slug(
            "btc-updown-5m-1234567890",
            "condition-123",
            1234567890,
            1234568190,
        );
        assert!(spec.is_some());
        let spec = spec.unwrap();
        assert_eq!(spec.asset, SupportedAsset::Btc);
        assert_eq!(spec.duration, SupportedDuration::M5);
    }

    #[test]
    fn extract_strike_from_slug() {
        assert_eq!(extract_strike("btc-above-85k"), Some(85000.0));
        assert_eq!(extract_strike("btc-above-85000"), Some(85000.0));
    }
}
