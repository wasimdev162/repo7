use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum InstrumentType {
    Spot,
    Perp,
    Option,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct InstrumentId {
    pub symbol: String,
    pub instrument_type: InstrumentType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Buy => 1.0,
            Side::Sell => -1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderType {
    Limit,
    Market,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MakerTaker {
    Maker,
    Taker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub instrument: InstrumentId,
    pub side: Side,
    pub qty: f64,
    pub price: Option<f64>,
    pub order_type: OrderType,
    pub post_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: Uuid,
    pub request: OrderRequest,
    pub status: OrderStatus,
    pub filled_qty: f64,
    pub create_ts: DateTime<Utc>,
    pub update_ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub order_id: Uuid,
    pub instrument: InstrumentId,
    pub side: Side,
    pub price: f64,
    pub qty: f64,
    pub ts: DateTime<Utc>,
    pub maker_taker: MakerTaker,
    pub mid_price: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: f64,
    pub size: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub instrument: InstrumentId,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub instrument: InstrumentId,
    pub price: f64,
    pub qty: f64,
    pub side: Side,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OrderAction {
    pub new_order: Option<OrderRequest>,
    pub cancel_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub fills: Vec<Fill>,
    pub cancelled: Vec<Uuid>,
}
