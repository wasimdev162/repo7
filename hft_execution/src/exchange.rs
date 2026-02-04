use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use chrono::Utc;
use rand::prelude::*;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time;
use uuid::Uuid;

use crate::config::SimulationConfig;
use crate::market_data::mid_price;
use crate::types::{
    BookLevel, BookSnapshot, Fill, InstrumentId, MakerTaker, Order, OrderRequest, OrderStatus,
    Side, Trade,
};

#[derive(Debug, Clone)]
pub enum MarketDataEvent {
    Book(BookSnapshot),
    Trade(Trade),
}

#[derive(Debug)]
pub enum ExchangeRequest {
    PlaceOrder {
        request: OrderRequest,
        resp: oneshot::Sender<Result<Order, ExchangeError>>,
    },
    CancelOrder {
        order_id: Uuid,
        resp: oneshot::Sender<Result<(), ExchangeError>>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExchangeError {
    #[error("post-only order would cross")]
    PostOnlyWouldCross,
    #[error("order not found")]
    OrderNotFound,
    #[error("exchange unavailable")]
    Unavailable,
}

#[derive(Debug)]
pub struct ExchangeHandle {
    request_tx: mpsc::Sender<ExchangeRequest>,
    market_tx: broadcast::Sender<MarketDataEvent>,
    fill_rx: mpsc::Receiver<Fill>,
}

impl ExchangeHandle {
    pub fn market_data_stream(&self) -> broadcast::Receiver<MarketDataEvent> {
        self.market_tx.subscribe()
    }

    pub async fn place_order(&self, request: OrderRequest) -> Result<Order, ExchangeError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let msg = ExchangeRequest::PlaceOrder {
            request,
            resp: resp_tx,
        };
        self.request_tx.send(msg).await.map_err(|_| ExchangeError::Unavailable)?;
        resp_rx.await.map_err(|_| ExchangeError::Unavailable)?
    }

    pub async fn cancel_order(&self, order_id: Uuid) -> Result<(), ExchangeError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let msg = ExchangeRequest::CancelOrder {
            order_id,
            resp: resp_tx,
        };
        self.request_tx.send(msg).await.map_err(|_| ExchangeError::Unavailable)?;
        resp_rx.await.map_err(|_| ExchangeError::Unavailable)?
    }

    pub fn fill_stream(&mut self) -> mpsc::Receiver<Fill> {
        std::mem::replace(&mut self.fill_rx, mpsc::channel(1).1)
    }
}

pub struct MockExchange {
    config: SimulationConfig,
    instruments: Vec<InstrumentId>,
}

impl MockExchange {
    pub fn new(config: SimulationConfig, instruments: Vec<InstrumentId>) -> Self {
        Self { config, instruments }
    }

    pub fn start(self) -> ExchangeHandle {
        let (request_tx, request_rx) = mpsc::channel(128);
        let (market_tx, _) = broadcast::channel(512);
        let (fill_tx, fill_rx) = mpsc::channel(512);

        let engine = MockExchangeEngine::new(self.config, self.instruments, request_rx, market_tx.clone(), fill_tx);
        tokio::spawn(async move { engine.run().await });

        ExchangeHandle {
            request_tx,
            market_tx,
            fill_rx,
        }
    }
}

struct SimInstrumentState {
    book: BookSnapshot,
    trades: VecDeque<Trade>,
    open_orders: HashMap<Uuid, Order>,
    rng: StdRng,
}

struct MockExchangeEngine {
    config: SimulationConfig,
    instruments: Vec<InstrumentId>,
    request_rx: mpsc::Receiver<ExchangeRequest>,
    market_tx: broadcast::Sender<MarketDataEvent>,
    fill_tx: mpsc::Sender<Fill>,
    states: HashMap<InstrumentId, SimInstrumentState>,
}

impl MockExchangeEngine {
    fn new(
        config: SimulationConfig,
        instruments: Vec<InstrumentId>,
        request_rx: mpsc::Receiver<ExchangeRequest>,
        market_tx: broadcast::Sender<MarketDataEvent>,
        fill_tx: mpsc::Sender<Fill>,
    ) -> Self {
        let mut states = HashMap::new();
        for instrument in &instruments {
            let mut rng = StdRng::seed_from_u64(config.seed);
            let snapshot = Self::initial_book(instrument.clone(), &config, &mut rng);
            states.insert(
                instrument.clone(),
                SimInstrumentState {
                    book: snapshot,
                    trades: VecDeque::new(),
                    open_orders: HashMap::new(),
                    rng,
                },
            );
        }

        Self {
            config,
            instruments,
            request_rx,
            market_tx,
            fill_tx,
            states,
        }
    }

    async fn run(mut self) {
        let mut interval = time::interval(Duration::from_millis(self.config.tick_interval_ms));
        let config = self.config.clone();
        let market_tx = self.market_tx.clone();
        let fill_tx = self.fill_tx.clone();
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    for instrument in &self.instruments {
                        if let Some(state) = self.states.get_mut(instrument) {
                            Self::update_market_state(&config, &market_tx, &fill_tx, state).await;
                        }
                    }
                }
                Some(req) = self.request_rx.recv() => {
                    self.handle_request(req).await;
                }
                else => break,
            }
        }
    }

    async fn handle_request(&mut self, req: ExchangeRequest) {
        match req {
            ExchangeRequest::PlaceOrder { request, resp } => {
                let result = self.place_order(request).await;
                let _ = resp.send(result);
            }
            ExchangeRequest::CancelOrder { order_id, resp } => {
                let result = self.cancel_order(order_id);
                let _ = resp.send(result);
            }
        }
    }

    async fn place_order(&mut self, request: OrderRequest) -> Result<Order, ExchangeError> {
        let state = self
            .states
            .get_mut(&request.instrument)
            .ok_or(ExchangeError::Unavailable)?;
        let best_bid = state.book.bids.first().map(|lvl| lvl.price).unwrap_or(0.0);
        let best_ask = state.book.asks.first().map(|lvl| lvl.price).unwrap_or(0.0);

        let crossing = match request.side {
            Side::Buy => request.price.unwrap_or(best_ask) >= best_ask,
            Side::Sell => request.price.unwrap_or(best_bid) <= best_bid,
        };

        if request.post_only && crossing {
            return Err(ExchangeError::PostOnlyWouldCross);
        }

        let now = Utc::now();
        let order = Order {
            id: Uuid::new_v4(),
            request: request.clone(),
            status: OrderStatus::New,
            filled_qty: 0.0,
            create_ts: now,
            update_ts: now,
        };

        state.open_orders.insert(order.id, order.clone());
        Ok(order)
    }

    fn cancel_order(&mut self, order_id: Uuid) -> Result<(), ExchangeError> {
        for state in self.states.values_mut() {
            if let Some(mut order) = state.open_orders.remove(&order_id) {
                order.status = OrderStatus::Cancelled;
                return Ok(());
            }
        }
        Err(ExchangeError::OrderNotFound)
    }

    async fn update_market_state(
        config: &SimulationConfig,
        market_tx: &broadcast::Sender<MarketDataEvent>,
        fill_tx: &mpsc::Sender<Fill>,
        state: &mut SimInstrumentState,
    ) {
        let now = Utc::now();
        let mid = mid_price(&state.book).unwrap_or(config.base_price);
        let drift = (state.rng.gen::<f64>() - 0.5) * config.price_volatility;
        let new_mid = (mid * (1.0 + drift)).max(0.01);
        let spread = (config.base_spread_bps / 10_000.0) * new_mid;
        let depth = config.depth_levels.max(1);

        let mut bids = Vec::with_capacity(depth);
        let mut asks = Vec::with_capacity(depth);
        for level in 0..depth {
            let step = spread * 0.2;
            let bid_price = new_mid - spread / 2.0 - step * level as f64;
            let ask_price = new_mid + spread / 2.0 + step * level as f64;
            let mut bid_size = config.level_liquidity * (1.0 - 0.05 * level as f64).max(0.2);
            let mut ask_size = config.level_liquidity * (1.0 - 0.05 * level as f64).max(0.2);

            if state.rng.gen::<f64>() < 0.05 {
                bid_size *= config.large_order_multiplier;
            }
            if state.rng.gen::<f64>() < 0.05 {
                ask_size *= config.large_order_multiplier;
            }

            bids.push(BookLevel {
                price: bid_price,
                size: bid_size,
            });
            asks.push(BookLevel {
                price: ask_price,
                size: ask_size,
            });
        }

        state.book = BookSnapshot {
            instrument: state.book.instrument.clone(),
            bids,
            asks,
            ts: now,
        };

        let _ = market_tx.send(MarketDataEvent::Book(state.book.clone()));

        let trade_count = (config.trade_rate_per_sec
            * (config.tick_interval_ms as f64 / 1000.0))
            .ceil() as usize;
        for _ in 0..trade_count {
            if state.rng.gen::<f64>() < 0.6 {
                let side = if state.rng.gen::<f64>() > 0.5 {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let price = if side == Side::Buy {
                    state.book.asks.first().map(|lvl| lvl.price).unwrap_or(new_mid)
                } else {
                    state.book.bids.first().map(|lvl| lvl.price).unwrap_or(new_mid)
                };
                let qty = (state.rng.gen::<f64>() * 0.8 + 0.2) * config.level_liquidity * 0.1;
                let trade = Trade {
                    instrument: state.book.instrument.clone(),
                    price,
                    qty,
                    side,
                    ts: now,
                };
                state.trades.push_back(trade.clone());
                let _ = market_tx.send(MarketDataEvent::Trade(trade));
            }
        }

        if state.rng.gen::<f64>() < config.iceberg_probability {
            let side = if state.rng.gen::<f64>() > 0.5 {
                Side::Buy
            } else {
                Side::Sell
            };
            let price = if side == Side::Buy {
                state.book.asks.first().map(|lvl| lvl.price).unwrap_or(new_mid)
            } else {
                state.book.bids.first().map(|lvl| lvl.price).unwrap_or(new_mid)
            };
            for _ in 0..5 {
                let trade = Trade {
                    instrument: state.book.instrument.clone(),
                    price,
                    qty: 0.2,
                    side,
                    ts: now,
                };
                state.trades.push_back(trade.clone());
                let _ = market_tx.send(MarketDataEvent::Trade(trade));
            }
        }

        Self::match_orders(state, fill_tx).await;
    }

    async fn match_orders(state: &mut SimInstrumentState, fill_tx: &mpsc::Sender<Fill>) {
        let now = Utc::now();
        let best_bid = state.book.bids.first().map(|lvl| lvl.price).unwrap_or(0.0);
        let best_ask = state.book.asks.first().map(|lvl| lvl.price).unwrap_or(0.0);
        let mid = (best_bid + best_ask) / 2.0;

        let mut filled_orders = Vec::new();
        for order in state.open_orders.values_mut() {
            let remaining = order.request.qty - order.filled_qty;
            if remaining <= 0.0 {
                order.status = OrderStatus::Filled;
                filled_orders.push(order.id);
                continue;
            }

            let crossing = match order.request.side {
                Side::Buy => order.request.price.unwrap_or(best_ask) >= best_ask,
                Side::Sell => order.request.price.unwrap_or(best_bid) <= best_bid,
            };

            let mut fill_now = false;
            let mut maker_taker = MakerTaker::Maker;
            if crossing {
                fill_now = true;
                maker_taker = MakerTaker::Taker;
            } else {
                let queue_ahead = if order.request.side == Side::Buy {
                    state
                        .book
                        .bids
                        .iter()
                        .find(|lvl| lvl.price <= order.request.price.unwrap_or(best_bid))
                        .map(|lvl| lvl.size)
                        .unwrap_or(0.0)
                } else {
                    state
                        .book
                        .asks
                        .iter()
                        .find(|lvl| lvl.price >= order.request.price.unwrap_or(best_ask))
                        .map(|lvl| lvl.size)
                        .unwrap_or(0.0)
                };
                let fill_prob = (0.15 * (1.0 / (1.0 + queue_ahead))).min(0.5);
                if state.rng.gen::<f64>() < fill_prob {
                    fill_now = true;
                }
            }

            if fill_now {
                let fill_qty = remaining.min(order.request.qty * 0.5).max(0.1);
                order.filled_qty += fill_qty;
                order.update_ts = now;
                order.status = if order.filled_qty >= order.request.qty {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartiallyFilled
                };

                let price = order.request.price.unwrap_or(mid);
                let fill = Fill {
                    order_id: order.id,
                    instrument: order.request.instrument.clone(),
                    side: order.request.side,
                    price,
                    qty: fill_qty,
                    ts: now,
                    maker_taker,
                    mid_price: mid,
                };
                let _ = fill_tx.send(fill).await;
            }
        }

        for order_id in filled_orders {
            state.open_orders.remove(&order_id);
        }
    }

    fn initial_book(
        instrument: InstrumentId,
        config: &SimulationConfig,
        rng: &mut StdRng,
    ) -> BookSnapshot {
        let mid = config.base_price;
        let spread = (config.base_spread_bps / 10_000.0) * mid;
        let depth = config.depth_levels.max(1);
        let mut bids = Vec::with_capacity(depth);
        let mut asks = Vec::with_capacity(depth);
        for level in 0..depth {
            let step = spread * 0.2;
            let bid_price = mid - spread / 2.0 - step * level as f64;
            let ask_price = mid + spread / 2.0 + step * level as f64;
            let mut bid_size = config.level_liquidity * (1.0 - 0.05 * level as f64).max(0.2);
            let mut ask_size = config.level_liquidity * (1.0 - 0.05 * level as f64).max(0.2);
            if rng.gen::<f64>() < 0.05 {
                bid_size *= config.large_order_multiplier;
            }
            if rng.gen::<f64>() < 0.05 {
                ask_size *= config.large_order_multiplier;
            }
            bids.push(BookLevel {
                price: bid_price,
                size: bid_size,
            });
            asks.push(BookLevel {
                price: ask_price,
                size: ask_size,
            });
        }

        BookSnapshot {
            instrument,
            bids,
            asks,
            ts: Utc::now(),
        }
    }
}

