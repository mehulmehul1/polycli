//! Pricing Module
//!
//! Fair value models and volatility calculations.

mod fair_value;
mod volatility;

pub use fair_value::{FairValueConfig, FairValueModel};
pub use volatility::{VolSurface, VolatilityCalculator};
