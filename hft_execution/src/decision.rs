use chrono::{DateTime, Utc};
use crate::config::ExecutionConfig;
use crate::market_data::MarketDataState;
use crate::signals::Signals;
use crate::types::{Order, OrderAction, OrderRequest, OrderType, Side};

#[derive(Debug, Clone)]
pub struct DecisionContext {
    pub remaining_qty: f64,
    pub open_orders: Vec<Order>,
    pub start_time: DateTime<Utc>,
    pub now: DateTime<Utc>,
    pub window_secs: u64,
}

#[derive(Debug, Clone)]
pub struct DecisionEngine {
    config: ExecutionConfig,
}

impl DecisionEngine {
    pub fn new(config: ExecutionConfig) -> Self {
        Self { config }
    }

    pub fn decide(
        &self,
        state: &MarketDataState,
        signals: &Signals,
        ctx: &DecisionContext,
        side: Side,
    ) -> OrderAction {
        let mut cancel_ids = Vec::new();
        let remaining = ctx.remaining_qty.max(0.0);
        let elapsed = (ctx.now - ctx.start_time).num_seconds().max(0) as f64;
        let window = ctx.window_secs as f64;
        let urgency = if window > 0.0 { elapsed / window } else { 1.0 };

        if remaining <= 0.0 {
            cancel_ids.extend(ctx.open_orders.iter().map(|order| order.id));
            return OrderAction {
                new_order: None,
                cancel_ids,
            };
        }

        if signals.toxic_flow && urgency < self.config.cross_spread_aggression {
            cancel_ids.extend(ctx.open_orders.iter().map(|order| order.id));
            return OrderAction {
                new_order: None,
                cancel_ids,
            };
        }

        let open_qty: f64 = ctx
            .open_orders
            .iter()
            .map(|order| order.request.qty - order.filled_qty)
            .sum();
        if open_qty >= remaining * 0.9 {
            return OrderAction {
                new_order: None,
                cancel_ids,
            };
        }

        let best_bid = state.best_bid();
        let best_ask = state.best_ask();
        if best_bid.is_none() || best_ask.is_none() {
            return OrderAction {
                new_order: None,
                cancel_ids,
            };
        }

        let best_bid = best_bid.unwrap();
        let best_ask = best_ask.unwrap();
        let spread_bps = signals.spread_bps;

        let imbalance_bias = signals.imbalance * side.sign();
        let flow_bias = signals.trade_flow_imbalance * side.sign();
        let momentum_bias = signals.momentum * side.sign();
        let mean_reversion_bias = signals.mean_reversion * side.sign();
        let directional_bias = imbalance_bias + flow_bias + momentum_bias + mean_reversion_bias;

        let momentum_against = match side {
            Side::Buy => signals.momentum > 0.0 && signals.trade_flow_imbalance > 0.0,
            Side::Sell => signals.momentum < 0.0 && signals.trade_flow_imbalance < 0.0,
        };

        let passive_price = match side {
            Side::Buy => best_bid.price,
            Side::Sell => best_ask.price,
        };
        let queue_ahead = state.estimate_queue_ahead(passive_price, side).unwrap_or(0.0);
        let queue_too_long = queue_ahead >= (best_bid.size + best_ask.size) * 1.5;

        let should_cross = urgency >= self.config.cross_spread_aggression
            || momentum_against
            || signals.iceberg_detected
            || directional_bias > 0.15
            || queue_too_long;

        let can_cross = spread_bps <= self.config.max_spread_bps_to_cross;

        let post_only = !should_cross
            && spread_bps >= self.config.post_only_spread_bps
            && !signals.iceberg_detected;

        let price = if should_cross && can_cross {
            Some(match side {
                Side::Buy => best_ask.price,
                Side::Sell => best_bid.price,
            })
        } else {
            Some(passive_price)
        };

        let order_type = if should_cross && can_cross {
            OrderType::Limit
        } else {
            OrderType::Limit
        };

        let top_liquidity = best_bid.size + best_ask.size;
        let mut child_qty = remaining.min(self.config.max_child_order_qty);
        child_qty = child_qty.max(self.config.min_child_order_qty);
        if top_liquidity > 0.0 {
            child_qty = child_qty.min(top_liquidity * 0.25);
        }
        if signals.large_resting_order {
            child_qty *= 0.7;
        }

        let new_order = OrderRequest {
            instrument: state.instrument.clone(),
            side,
            qty: child_qty,
            price,
            order_type,
            post_only,
        };

        OrderAction {
            new_order: Some(new_order),
            cancel_ids,
        }
    }
}
