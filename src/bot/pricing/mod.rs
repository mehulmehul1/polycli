//! Pricing Module
//!
//! Fair value models and volatility calculations.

mod fair_value;
mod volatility;

pub use fair_value::{FairValueModel, FairValueConfig};
pub use volatility::{VolatilityCalculator, VolSurface};
