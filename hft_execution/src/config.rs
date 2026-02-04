use serde::{Deserialize, Serialize};

use crate::types::{InstrumentId, Side};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub runtime: RuntimeConfig,
    pub logging: LoggingConfig,
    pub instruments: Vec<InstrumentConfig>,
    pub execution: ExecutionConfig,
    pub simulation: SimulationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub mode: RuntimeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeMode {
    Simulation,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub log_level: String,
    pub csv_fill_log_path: String,
    pub report_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub execution_window_secs: u64,
    pub decision_interval_ms: u64,
    pub max_child_order_qty: f64,
    pub min_child_order_qty: f64,
    pub order_ttl_ms: u64,
    pub post_only_spread_bps: f64,
    pub cross_spread_aggression: f64,
    pub max_spread_bps_to_cross: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentConfig {
    pub instrument: InstrumentId,
    pub side: Side,
    pub target_qty: f64,
    pub decision_price_override: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    pub enabled: bool,
    pub seed: u64,
    pub tick_interval_ms: u64,
    pub base_price: f64,
    pub price_volatility: f64,
    pub base_spread_bps: f64,
    pub depth_levels: usize,
    pub level_liquidity: f64,
    pub trade_rate_per_sec: f64,
    pub iceberg_probability: f64,
    pub large_order_multiplier: f64,
}

