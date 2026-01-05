use chrono::{DateTime, Utc};
use crate::{OrderBook, OrderData};

pub struct OrderBookEvent {
    pub instrument_id: i16,
    pub order_book: OrderBook,
}

pub struct OrderBookDeltaEvent {
    pub instrument_id: i16,
    pub timestamp: DateTime<Utc>,
    pub asks: Vec<OrderData>,
    pub bids: Vec<OrderData>,
}