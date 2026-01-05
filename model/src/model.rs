pub mod utils;
pub mod events;
pub mod common;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Instrument {
    pub id: Option<i16>,
    pub symbol: String,
    pub status: String,
    pub base_coin: String,
    pub quote_coin: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderBook {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub asks: Vec<OrderData>,
    pub bids: Vec<OrderData>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OrderData {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OrderDataDelta {
    pub price_id: i32,
    pub size: Decimal,
}

#[derive(Clone, Debug)]
pub struct OrderDataSlice {
    pub timestamp: DateTime<Utc>,
    pub order_data: Vec<OrderData>,
}
