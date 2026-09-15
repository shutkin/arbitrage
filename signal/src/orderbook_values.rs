use chrono::{DateTime, Utc};

pub const IMBALANCE_LEVELS: [usize; 5] = [3, 5, 10, 20, 50];

pub struct OrderBookValues {
    pub time: DateTime<Utc>,
    pub ask: f64,
    pub bid: f64,
    pub imbalances: [f64; IMBALANCE_LEVELS.len()],
}