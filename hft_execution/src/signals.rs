use crate::market_data::MarketDataState;

#[derive(Debug, Clone)]
pub struct Signals {
    pub imbalance: f64,
    pub trade_flow_imbalance: f64,
    pub momentum: f64,
    pub mean_reversion: f64,
    pub spread_bps: f64,
    pub large_resting_order: bool,
    pub iceberg_detected: bool,
    pub toxic_flow: bool,
}

impl Signals {
    pub fn from_market_data(state: &MarketDataState) -> Self {
        let imbalance = state.imbalance(5).unwrap_or(0.0);
        let trade_flow_imbalance = state.trade_flow_imbalance().unwrap_or(0.0);
        let momentum = state.short_term_momentum().unwrap_or(0.0);
        let mean_reversion = state.mean_reversion_after_large_trade(5.0).unwrap_or(0.0);
        let spread_bps = state.spread_bps().unwrap_or(0.0);
        let large_resting_order = state.has_large_resting_order(2.5);
        let iceberg_detected = state.detect_iceberg(5, 0.5);
        let toxic_flow = trade_flow_imbalance.abs() > 0.4 && spread_bps > 8.0;

        Self {
            imbalance,
            trade_flow_imbalance,
            momentum,
            mean_reversion,
            spread_bps,
            large_resting_order,
            iceberg_detected,
            toxic_flow,
        }
    }
}
