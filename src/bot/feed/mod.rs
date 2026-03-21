//! Market Feed Module
//!
//! Provides WebSocket and REST-based feeds for Polymarket market data.

pub mod multi_market_feed;

// Re-export common types from the feed_base for compatibility
pub use crate::bot::feed_base::{
    BookDeltaEvent, BookChangeSide, DualBookState, DualSnapshot, LiveFeedMode,
    LiveStrategyInputSource, MarketSnapshot, MarketWebsocketFeed, OutcomeSide,
    PollingSnapshotSource, ReplayMode, ReplaySnapshotSource, StrategyInputSource,
    WebsocketSnapshotSource, parse_market_ws_value,
};

pub use multi_market_feed::{
    MarketEvent, MarketSubscription, MultiMarketWebsocketFeed, MultiMarketAggregator,
    FeedStats,
};
