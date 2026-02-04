use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::{ExecutionConfig, InstrumentConfig};
use crate::decision::{DecisionContext, DecisionEngine};
use crate::exchange::{ExchangeHandle, MarketDataEvent};
use crate::market_data::MarketDataState;
use crate::performance::PerformanceTracker;
use crate::signals::Signals;
use crate::types::{Fill, InstrumentId, Order, OrderStatus};

pub struct ExecutionEngine {
    exchange: ExchangeHandle,
    decision_engine: DecisionEngine,
    performance: PerformanceTracker,
    execution_config: ExecutionConfig,
    instruments: Vec<InstrumentConfig>,
}

struct InstrumentRuntime {
    config: InstrumentConfig,
    start_time: DateTime<Utc>,
    remaining_qty: f64,
    open_orders: HashMap<Uuid, Order>,
}

impl ExecutionEngine {
    pub fn new(
        exchange: ExchangeHandle,
        decision_engine: DecisionEngine,
        performance: PerformanceTracker,
        execution_config: ExecutionConfig,
        instruments: Vec<InstrumentConfig>,
    ) -> Self {
        Self {
            exchange,
            decision_engine,
            performance,
            execution_config,
            instruments,
        }
    }

    pub async fn run(mut self) -> Vec<crate::performance::InstrumentReport> {
        let mut market_rx = self.exchange.market_data_stream();
        let mut fill_rx = self.exchange.fill_stream();

        let mut states: HashMap<InstrumentId, MarketDataState> = HashMap::new();
        let mut runtime: HashMap<InstrumentId, InstrumentRuntime> = HashMap::new();
        let start_time = Utc::now();

        while states.len() < self.instruments.len() {
            if let Ok(event) = market_rx.recv().await {
                if let MarketDataEvent::Book(snapshot) = event {
                    states
                        .entry(snapshot.instrument.clone())
                        .or_insert_with(|| MarketDataState::new(snapshot));
                }
            }
        }

        for instrument_cfg in &self.instruments {
            if let Some(state) = states.get(&instrument_cfg.instrument) {
                let decision_price = instrument_cfg
                    .decision_price_override
                    .or_else(|| state.mid_price())
                    .unwrap_or(0.0);
                self.performance.register_instrument(
                    instrument_cfg.instrument.clone(),
                    instrument_cfg.side,
                    instrument_cfg.target_qty,
                    decision_price,
                );

                runtime.insert(
                    instrument_cfg.instrument.clone(),
                    InstrumentRuntime {
                        config: instrument_cfg.clone(),
                        start_time,
                        remaining_qty: instrument_cfg.target_qty,
                        open_orders: HashMap::new(),
                    },
                );
            }
        }

        let mut decision_interval =
            time::interval(Duration::from_millis(self.execution_config.decision_interval_ms));
        let deadline = start_time + chrono::Duration::seconds(self.execution_config.execution_window_secs as i64);
        let mut finished = false;

        loop {
            tokio::select! {
                _ = decision_interval.tick() => {
                    let now = Utc::now();
                    if now >= deadline {
                        finished = true;
                    }
                    self.handle_decisions(&states, &mut runtime, now, finished).await;
                    if finished && runtime.values().all(|rt| rt.remaining_qty <= 0.0 && rt.open_orders.is_empty()) {
                        break;
                    }
                }
                Ok(event) = market_rx.recv() => {
                    self.handle_market_event(event, &mut states).await;
                }
                Some(fill) = fill_rx.recv() => {
                    self.handle_fill(fill, &mut runtime);
                }
            }
        }

        self.performance.finalize()
    }

    async fn handle_market_event(
        &mut self,
        event: MarketDataEvent,
        states: &mut HashMap<InstrumentId, MarketDataState>,
    ) {
        match event {
            MarketDataEvent::Book(snapshot) => {
                let mid = snapshot
                    .bids
                    .first()
                    .zip(snapshot.asks.first())
                    .map(|(bid, ask)| (bid.price + ask.price) / 2.0)
                    .unwrap_or(0.0);
                states
                    .entry(snapshot.instrument.clone())
                    .and_modify(|state| state.update_book(snapshot.clone()))
                    .or_insert_with(|| MarketDataState::new(snapshot.clone()));
                self.performance
                    .update_mid_price(&snapshot.instrument, mid, snapshot.ts);
            }
            MarketDataEvent::Trade(trade) => {
                if let Some(state) = states.get_mut(&trade.instrument) {
                    state.record_trade(trade.clone(), 30);
                }
                self.performance.record_trade(&trade);
            }
        }
    }

    fn handle_fill(&mut self, fill: Fill, runtime: &mut HashMap<InstrumentId, InstrumentRuntime>) {
        if let Some(rt) = runtime.get_mut(&fill.instrument) {
            rt.remaining_qty = (rt.remaining_qty - fill.qty).max(0.0);
            if let Some(order) = rt.open_orders.get_mut(&fill.order_id) {
                order.filled_qty += fill.qty;
                if order.filled_qty >= order.request.qty {
                    order.status = OrderStatus::Filled;
                } else {
                    order.status = OrderStatus::PartiallyFilled;
                }
            }
        }
        self.performance.record_fill(&fill);
    }

    async fn handle_decisions(
        &mut self,
        states: &HashMap<InstrumentId, MarketDataState>,
        runtime: &mut HashMap<InstrumentId, InstrumentRuntime>,
        now: DateTime<Utc>,
        finished: bool,
    ) {
        let ttl = chrono::Duration::milliseconds(self.execution_config.order_ttl_ms as i64);

        for (instrument, rt) in runtime.iter_mut() {
            if let Some(state) = states.get(instrument) {
                let mut cancel_ids = Vec::new();
                for (order_id, order) in rt.open_orders.iter() {
                    if now - order.create_ts >= ttl {
                        cancel_ids.push(*order_id);
                    }
                }

                for order_id in cancel_ids {
                    if let Err(err) = self.exchange.cancel_order(order_id).await {
                        warn!("cancel failed: {:?}", err);
                    } else {
                        rt.open_orders.remove(&order_id);
                    }
                }

                if finished || rt.remaining_qty <= 0.0 {
                    continue;
                }

                let signals = Signals::from_market_data(state);
                let context = DecisionContext {
                    remaining_qty: rt.remaining_qty,
                    open_orders: rt.open_orders.values().cloned().collect(),
                    start_time: rt.start_time,
                    now,
                    window_secs: self.execution_config.execution_window_secs,
                };

                let action = self
                    .decision_engine
                    .decide(state, &signals, &context, rt.config.side);

                for order_id in action.cancel_ids {
                    if let Err(err) = self.exchange.cancel_order(order_id).await {
                        warn!("cancel failed: {:?}", err);
                    } else {
                        rt.open_orders.remove(&order_id);
                    }
                }

                if let Some(new_order) = action.new_order {
                    match self.exchange.place_order(new_order).await {
                        Ok(order) => {
                            rt.open_orders.insert(order.id, order);
                        }
                        Err(err) => {
                            info!("order rejected: {:?}", err);
                        }
                    }
                }
            }
        }
    }
}
