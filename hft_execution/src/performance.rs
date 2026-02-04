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
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::types::{Fill, InstrumentId, InstrumentType, MakerTaker, Side, Trade};

#[derive(Debug, Clone)]
pub struct DecisionInfo {
    pub decision_price: f64,
    pub side: Side,
    pub target_qty: f64,
}

#[derive(Debug, Clone)]
struct PendingAdverse {
    fill_id: Uuid,
    fill_ts: DateTime<Utc>,
    fill_price: f64,
    side: Side,
    mid_at_fill: f64,
    one_sec: Option<f64>,
    five_sec: Option<f64>,
    thirty_sec: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct InstrumentMetrics {
    pub instrument_type: InstrumentType,
    pub implementation_shortfall_bps: f64,
    pub vwap_outperformance_bps: f64,
    pub fill_rate: f64,
    pub maker_ratio: f64,
    pub adverse_selection_1s: f64,
    pub adverse_selection_5s: f64,
    pub adverse_selection_30s: f64,
}

#[derive(Debug)]
pub struct PerformanceTracker {
    decisions: HashMap<InstrumentId, DecisionInfo>,
    fills: Vec<Fill>,
    trades: Vec<Trade>,
    pending_adverse: Vec<PendingAdverse>,
    adverse_samples: HashMap<Uuid, PendingAdverse>,
}

impl PerformanceTracker {
    pub fn new() -> Self {
        Self {
            decisions: HashMap::new(),
            fills: Vec::new(),
            trades: Vec::new(),
            pending_adverse: Vec::new(),
            adverse_samples: HashMap::new(),
        }
    }

    pub fn record_decision(&mut self, instrument: InstrumentId, decision: DecisionInfo) {
        self.decisions.insert(instrument, decision);
    }

    pub fn on_fill(&mut self, fill: Fill) {
        let pending = PendingAdverse {
            fill_id: fill.order_id,
            fill_ts: fill.ts,
            fill_price: fill.price,
            side: fill.side,
            mid_at_fill: fill.mid_price,
            one_sec: None,
            five_sec: None,
            thirty_sec: None,
        };
        self.pending_adverse.push(pending);
        self.fills.push(fill);
    }

    pub fn on_trade(&mut self, trade: Trade) {
        self.trades.push(trade);
    }

    pub fn on_mid_price(&mut self, now: DateTime<Utc>, mid: f64) {
        let mut completed = Vec::new();
        for pending in &mut self.pending_adverse {
            let age = now - pending.fill_ts;
            if pending.one_sec.is_none() && age >= Duration::seconds(1) {
                pending.one_sec = Some(price_move_bps(pending.fill_price, mid, pending.side));
            }
            if pending.five_sec.is_none() && age >= Duration::seconds(5) {
                pending.five_sec = Some(price_move_bps(pending.fill_price, mid, pending.side));
            }
            if pending.thirty_sec.is_none() && age >= Duration::seconds(30) {
                pending.thirty_sec = Some(price_move_bps(pending.fill_price, mid, pending.side));
            }
            if pending.one_sec.is_some()
                && pending.five_sec.is_some()
                && pending.thirty_sec.is_some()
            {
                completed.push(pending.fill_id);
            }
        }

        for fill_id in completed {
            if let Some(index) = self
                .pending_adverse
                .iter()
                .position(|pending| pending.fill_id == fill_id)
            {
                let pending = self.pending_adverse.remove(index);
                self.adverse_samples.insert(fill_id, pending);
            }
        }
    }

    pub fn metrics_by_instrument_type(&self) -> Vec<InstrumentMetrics> {
        let mut metrics_map: HashMap<InstrumentType, Vec<InstrumentId>> = HashMap::new();
        for instrument in self.decisions.keys() {
            metrics_map
                .entry(instrument.instrument_type)
                .or_default()
                .push(instrument.clone());
        }

        metrics_map
            .into_iter()
            .map(|(instrument_type, instruments)| {
                let mut fills = Vec::new();
                let mut trades = Vec::new();
                for fill in &self.fills {
                    if instruments.contains(&fill.instrument) {
                        fills.push(fill);
                    }
                }
                for trade in &self.trades {
                    if instruments.contains(&trade.instrument) {
                        trades.push(trade);
                    }
                }

                let implementation_shortfall_bps =
                    self.compute_is_bps(&instruments, &fills).unwrap_or(0.0);
                let vwap_outperformance_bps = compute_vwap_outperformance(&fills, &trades);
                let fill_rate = self.compute_fill_rate(&instruments, &fills).unwrap_or(0.0);
                let maker_ratio = compute_maker_ratio(&fills);
                let adverse_selection_1s = self.adverse_selection(&fills, 1);
                let adverse_selection_5s = self.adverse_selection(&fills, 5);
                let adverse_selection_30s = self.adverse_selection(&fills, 30);

                InstrumentMetrics {
                    instrument_type,
                    implementation_shortfall_bps,
                    vwap_outperformance_bps,
                    fill_rate,
                    maker_ratio,
                    adverse_selection_1s,
                    adverse_selection_5s,
                    adverse_selection_30s,
                }
            })
            .collect()
    }

    fn compute_is_bps(
        &self,
        instruments: &[InstrumentId],
        fills: &[&Fill],
    ) -> Option<f64> {
        let mut total = 0.0;
        let mut qty = 0.0;
        for fill in fills {
            let decision = self.decisions.get(&fill.instrument)?;
            let signed = (fill.price - decision.decision_price) / decision.decision_price;
            let signed = signed * fill.side.sign();
            total += signed * fill.qty;
            qty += fill.qty;
        }
        if qty == 0.0 {
            return None;
        }
        Some((total / qty) * 10_000.0)
    }

    fn compute_fill_rate(
        &self,
        instruments: &[InstrumentId],
        fills: &[&Fill],
    ) -> Option<f64> {
        let mut target = 0.0;
        for instrument in instruments {
            if let Some(decision) = self.decisions.get(instrument) {
                target += decision.target_qty;
            }
        }
        let executed: f64 = fills.iter().map(|fill| fill.qty).sum();
        if target == 0.0 {
            return None;
        }
        Some(executed / target)
    }

    fn adverse_selection(&self, fills: &[&Fill], horizon_sec: i64) -> f64 {
        let mut total = 0.0;
        let mut count = 0.0;
        for fill in fills {
            if let Some(sample) = self.adverse_samples.get(&fill.order_id) {
                let value = match horizon_sec {
                    1 => sample.one_sec,
                    5 => sample.five_sec,
                    30 => sample.thirty_sec,
                    _ => None,
                };
                if let Some(val) = value {
                    total += val;
                    count += 1.0;
                }
            }
        }
        if count == 0.0 {
            0.0
        } else {
            total / count
        }
    }
}

fn price_move_bps(fill_price: f64, new_mid: f64, side: Side) -> f64 {
    if fill_price <= 0.0 {
        return 0.0;
    }
    let raw = (new_mid - fill_price) / fill_price;
    (raw * side.sign()) * 10_000.0
}

fn compute_vwap_outperformance(fills: &[&Fill], trades: &[Trade]) -> f64 {
    let exec_vwap = compute_vwap_from_fills(fills).unwrap_or(0.0);
    let market_vwap = compute_vwap_from_trades(trades).unwrap_or(0.0);
    if market_vwap == 0.0 {
        return 0.0;
    }
    ((exec_vwap - market_vwap) / market_vwap) * 10_000.0
}

fn compute_vwap_from_fills(fills: &[&Fill]) -> Option<f64> {
    let mut notional = 0.0;
    let mut qty = 0.0;
    for fill in fills {
        notional += fill.price * fill.qty;
        qty += fill.qty;
    }
    if qty == 0.0 {
        None
    } else {
        Some(notional / qty)
    }
}

fn compute_vwap_from_trades(trades: &[Trade]) -> Option<f64> {
    let mut notional = 0.0;
    let mut qty = 0.0;
    for trade in trades {
        notional += trade.price * trade.qty;
        qty += trade.qty;
    }
    if qty == 0.0 {
        None
    } else {
        Some(notional / qty)
    }
}

fn compute_maker_ratio(fills: &[&Fill]) -> f64 {
    let mut maker = 0.0;
    let mut total = 0.0;
    for fill in fills {
        if fill.maker_taker == MakerTaker::Maker {
            maker += fill.qty;
        }
        total += fill.qty;
    }
    if total == 0.0 {
        0.0
    } else {
        maker / total
    }
}
