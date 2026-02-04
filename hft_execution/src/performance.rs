use std::collections::HashMap;
use std::fs::File;

use chrono::{DateTime, Duration, Utc};
use csv::Writer;
use serde::Serialize;

use crate::types::{Fill, InstrumentId, MakerTaker, Side, Trade};

#[derive(Debug)]
pub struct PerformanceTracker {
    instruments: HashMap<InstrumentId, InstrumentMetrics>,
    fill_writer: Writer<File>,
}

#[derive(Debug)]
pub struct InstrumentMetrics {
    pub instrument: InstrumentId,
    pub side: Side,
    pub decision_price: f64,
    pub target_qty: f64,
    pub executed_qty: f64,
    pub execution_value: f64,
    pub maker_qty: f64,
    pub taker_qty: f64,
    pub market_trade_qty: f64,
    pub market_trade_value: f64,
    pub adverse_pending: Vec<PendingAdverse>,
    pub adverse_sum: [f64; 3],
    pub adverse_count: [u64; 3],
}

#[derive(Debug)]
pub struct PendingAdverse {
    pub ts: DateTime<Utc>,
    pub mid_price: f64,
    pub side: Side,
    pub horizons: [Duration; 3],
    pub recorded: [bool; 3],
}

#[derive(Debug, Serialize)]
pub struct InstrumentReport {
    pub instrument: InstrumentId,
    pub side: Side,
    pub decision_price: f64,
    pub avg_execution_price: f64,
    pub implementation_shortfall_bps: f64,
    pub vwap_bps: f64,
    pub fill_rate: f64,
    pub maker_ratio: f64,
    pub taker_ratio: f64,
    pub adverse_1s_bps: f64,
    pub adverse_5s_bps: f64,
    pub adverse_30s_bps: f64,
}

impl PerformanceTracker {
    pub fn new(csv_fill_log_path: &str) -> Result<Self, std::io::Error> {
        let file = File::create(csv_fill_log_path)?;
        let mut writer = Writer::from_writer(file);
        writer.write_record([
            "timestamp",
            "instrument",
            "instrument_type",
            "side",
            "price",
            "qty",
            "maker_taker",
            "mid_price",
        ])?;

        Ok(Self {
            instruments: HashMap::new(),
            fill_writer: writer,
        })
    }

    pub fn register_instrument(
        &mut self,
        instrument: InstrumentId,
        side: Side,
        target_qty: f64,
        decision_price: f64,
    ) {
        self.instruments.insert(
            instrument.clone(),
            InstrumentMetrics {
                instrument,
                side,
                decision_price,
                target_qty,
                executed_qty: 0.0,
                execution_value: 0.0,
                maker_qty: 0.0,
                taker_qty: 0.0,
                market_trade_qty: 0.0,
                market_trade_value: 0.0,
                adverse_pending: Vec::new(),
                adverse_sum: [0.0; 3],
                adverse_count: [0; 3],
            },
        );
    }

    pub fn record_trade(&mut self, trade: &Trade) {
        if let Some(metrics) = self.instruments.get_mut(&trade.instrument) {
            metrics.market_trade_qty += trade.qty;
            metrics.market_trade_value += trade.price * trade.qty;
        }
    }

    pub fn record_fill(&mut self, fill: &Fill) {
        if let Some(metrics) = self.instruments.get_mut(&fill.instrument) {
            metrics.executed_qty += fill.qty;
            metrics.execution_value += fill.price * fill.qty;
            match fill.maker_taker {
                MakerTaker::Maker => metrics.maker_qty += fill.qty,
                MakerTaker::Taker => metrics.taker_qty += fill.qty,
            }

            metrics.adverse_pending.push(PendingAdverse {
                ts: fill.ts,
                mid_price: fill.mid_price,
                side: fill.side,
                horizons: [
                    Duration::seconds(1),
                    Duration::seconds(5),
                    Duration::seconds(30),
                ],
                recorded: [false; 3],
            });
        }

        let _ = self.fill_writer.serialize((
            fill.ts.to_rfc3339(),
            fill.instrument.symbol.clone(),
            format!("{:?}", fill.instrument.instrument_type),
            format!("{:?}", fill.side),
            fill.price,
            fill.qty,
            format!("{:?}", fill.maker_taker),
            fill.mid_price,
        ));
    }

    pub fn update_mid_price(&mut self, instrument: &InstrumentId, mid: f64, now: DateTime<Utc>) {
        if let Some(metrics) = self.instruments.get_mut(instrument) {
            for pending in &mut metrics.adverse_pending {
                let elapsed = now - pending.ts;
                for idx in 0..pending.horizons.len() {
                    if !pending.recorded[idx] && elapsed >= pending.horizons[idx] {
                        let movement_bps = if pending.mid_price > 0.0 {
                            (mid - pending.mid_price) / pending.mid_price * 10_000.0
                        } else {
                            0.0
                        };
                        metrics.adverse_sum[idx] += movement_bps;
                        metrics.adverse_count[idx] += 1;
                        pending.recorded[idx] = true;
                    }
                }
            }

            metrics
                .adverse_pending
                .retain(|pending| pending.recorded.iter().any(|done| !done));
        }
    }

    pub fn finalize(mut self) -> Vec<InstrumentReport> {
        let _ = self.fill_writer.flush();
        self.instruments
            .values()
            .map(|metrics| InstrumentReport {
                instrument: metrics.instrument.clone(),
                side: metrics.side,
                decision_price: metrics.decision_price,
                avg_execution_price: if metrics.executed_qty > 0.0 {
                    metrics.execution_value / metrics.executed_qty
                } else {
                    0.0
                },
                implementation_shortfall_bps: compute_is_bps(metrics),
                vwap_bps: compute_vwap_bps(metrics),
                fill_rate: if metrics.target_qty > 0.0 {
                    metrics.executed_qty / metrics.target_qty
                } else {
                    0.0
                },
                maker_ratio: ratio(metrics.maker_qty, metrics.executed_qty),
                taker_ratio: ratio(metrics.taker_qty, metrics.executed_qty),
                adverse_1s_bps: avg_adverse(metrics, 0),
                adverse_5s_bps: avg_adverse(metrics, 1),
                adverse_30s_bps: avg_adverse(metrics, 2),
            })
            .collect()
    }
}

fn compute_is_bps(metrics: &InstrumentMetrics) -> f64 {
    if metrics.executed_qty <= 0.0 || metrics.decision_price <= 0.0 {
        return 0.0;
    }
    let avg_price = metrics.execution_value / metrics.executed_qty;
    ((avg_price - metrics.decision_price) / metrics.decision_price) * 10_000.0
        * metrics.side.sign()
}

fn compute_vwap_bps(metrics: &InstrumentMetrics) -> f64 {
    if metrics.executed_qty <= 0.0 || metrics.market_trade_qty <= 0.0 {
        return 0.0;
    }
    let avg_price = metrics.execution_value / metrics.executed_qty;
    let market_vwap = metrics.market_trade_value / metrics.market_trade_qty;
    if market_vwap <= 0.0 {
        return 0.0;
    }
    ((avg_price - market_vwap) / market_vwap) * 10_000.0 * metrics.side.sign()
}

fn avg_adverse(metrics: &InstrumentMetrics, idx: usize) -> f64 {
    if metrics.adverse_count[idx] == 0 {
        return 0.0;
    }
    metrics.adverse_sum[idx] / metrics.adverse_count[idx] as f64
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator <= 0.0 {
        return 0.0;
    }
    numerator / denominator
}
