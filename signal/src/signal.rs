mod orderbook_values;

use model::OrderBook;

pub struct Signal {
    ticker1: String,
    ticker2: String,
}

impl Signal {
    pub fn process(&self, ticker: &str, order_book: OrderBook) {
        
    }
}