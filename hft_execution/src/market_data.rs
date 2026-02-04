use std::collections::VecDeque;

use chrono::{Duration, Utc};

use crate::types::{BookLevel, BookSnapshot, InstrumentId, Side, Trade};

#[derive(Debug, Clone)]
pub struct MarketDataState {
    pub instrument: InstrumentId,
    pub book: BookSnapshot,
    pub trades: VecDeque<Trade>,
    pub last_mid: f64,
}

impl MarketDataState {
    pub fn new(snapshot: BookSnapshot) -> Self {
        let mid = mid_price(&snapshot).unwrap_or(0.0);
        Self {
            instrument: snapshot.instrument.clone(),
            book: snapshot,
            trades: VecDeque::new(),
            last_mid: mid,
        }
    }

    pub fn update_book(&mut self, snapshot: BookSnapshot) {
        self.last_mid = mid_price(&snapshot).unwrap_or(self.last_mid);
        self.book = snapshot;
    }

    pub fn record_trade(&mut self, trade: Trade, window_secs: i64) {
        self.trades.push_back(trade);
        self.prune_trades(window_secs);
    }

    pub fn prune_trades(&mut self, window_secs: i64) {
        let cutoff = Utc::now() - Duration::seconds(window_secs);
        while let Some(front) = self.trades.front() {
            if front.ts < cutoff {
                self.trades.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn best_bid(&self) -> Option<BookLevel> {
        self.book.bids.first().cloned()
    }

    pub fn best_ask(&self) -> Option<BookLevel> {
        self.book.asks.first().cloned()
    }

    pub fn mid_price(&self) -> Option<f64> {
        mid_price(&self.book)
    }

    pub fn spread_bps(&self) -> Option<f64> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        if bid.price <= 0.0 {
            return None;
        }
        Some(((ask.price - bid.price) / bid.price) * 10_000.0)
    }

    pub fn imbalance(&self, top_levels: usize) -> Option<f64> {
        let bid_sum: f64 = self
            .book
            .bids
            .iter()
            .take(top_levels)
            .map(|lvl| lvl.size)
            .sum();
        let ask_sum: f64 = self
            .book
            .asks
            .iter()
            .take(top_levels)
            .map(|lvl| lvl.size)
            .sum();
        let total = bid_sum + ask_sum;
        if total == 0.0 {
            return None;
        }
        Some((bid_sum - ask_sum) / total)
    }

    pub fn trade_flow_imbalance(&self) -> Option<f64> {
        let mut buy = 0.0;
        let mut sell = 0.0;
        for trade in &self.trades {
            match trade.side {
                Side::Buy => buy += trade.qty,
                Side::Sell => sell += trade.qty,
            }
        }
        let total = buy + sell;
        if total == 0.0 {
            return None;
        }
        Some((buy - sell) / total)
    }

    pub fn short_term_momentum(&self) -> Option<f64> {
        let first = self.trades.front()?;
        let last = self.trades.back()?;
        if first.price <= 0.0 {
            return None;
        }
        Some((last.price - first.price) / first.price)
    }

    pub fn mean_reversion_after_large_trade(&self, threshold_qty: f64) -> Option<f64> {
        let large_trade = self
            .trades
            .iter()
            .rev()
            .find(|trade| trade.qty >= threshold_qty)?;
        let last = self.trades.back()?;
        if large_trade.price <= 0.0 {
            return None;
        }
        Some((last.price - large_trade.price) / large_trade.price)
    }

    pub fn has_large_resting_order(&self, multiplier: f64) -> bool {
        let avg_bid = average_size(&self.book.bids);
        let avg_ask = average_size(&self.book.asks);
        let bid_large = self
            .book
            .bids
            .iter()
            .any(|lvl| lvl.size >= avg_bid * multiplier);
        let ask_large = self
            .book
            .asks
            .iter()
            .any(|lvl| lvl.size >= avg_ask * multiplier);
        bid_large || ask_large
    }

    pub fn detect_iceberg(&self, min_trade_count: usize, max_trade_size: f64) -> bool {
        let mut price_counts = std::collections::HashMap::new();
        for trade in &self.trades {
            if trade.qty <= max_trade_size {
                let count = price_counts.entry(trade.price.to_string()).or_insert(0);
                *count += 1;
            }
        }
        price_counts.values().any(|count| *count >= min_trade_count)
    }

    pub fn estimate_queue_ahead(&self, price: f64, side: Side) -> Option<f64> {
        match side {
            Side::Buy => {
                for lvl in &self.book.bids {
                    if (lvl.price - price).abs() < f64::EPSILON {
                        return Some(lvl.size);
                    }
                    if lvl.price < price {
                        break;
                    }
                }
            }
            Side::Sell => {
                for lvl in &self.book.asks {
                    if (lvl.price - price).abs() < f64::EPSILON {
                        return Some(lvl.size);
                    }
                    if lvl.price > price {
                        break;
                    }
                }
            }
        }
        None
    }
}

pub fn mid_price(snapshot: &BookSnapshot) -> Option<f64> {
    let bid = snapshot.bids.first()?;
    let ask = snapshot.asks.first()?;
    Some((bid.price + ask.price) / 2.0)
}

fn average_size(levels: &[BookLevel]) -> f64 {
    if levels.is_empty() {
        return 0.0;
    }
    let sum: f64 = levels.iter().map(|lvl| lvl.size).sum();
    sum / levels.len() as f64
}
