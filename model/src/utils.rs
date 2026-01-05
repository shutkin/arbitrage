use std::fs::File;
use std::io::Write;
use chrono::{DateTime, Timelike, Utc};
use rust_decimal::Decimal;
use crate::{OrderBook, OrderData};

pub fn calculate_delta_sell(order_book1: &OrderBook, order_book2: &OrderBook, min_volume: Decimal) -> Option<OrderData> {
    let max_bid = order_book1.bids.iter()
        .filter(|bid| bid.price * bid.size > min_volume)
        .max_by_key(|bid| bid.price);
    let min_ask = order_book2.asks.iter()
        .filter(|ask| ask.price * ask.size > min_volume)
        .min_by_key(|ask| ask.price);
    map_delta(max_bid, min_ask)
}

pub fn calculate_delta_buy(order_book1: &OrderBook, order_book2: &OrderBook, min_volume: Decimal) -> Option<OrderData> {
    let min_ask = order_book1.asks.iter()
        .filter(|ask| ask.price * ask.size > min_volume)
        .min_by_key(|ask| ask.price);
    let max_bid = order_book2.bids.iter()
        .filter(|bid| bid.price * bid.size > min_volume)
        .max_by_key(|bid| bid.price);
    map_delta(min_ask, max_bid)
}

fn map_delta(data1: Option<&OrderData>, data2: Option<&OrderData>) -> Option<OrderData> {
    if let (Some(data1), Some(data2)) = (data1, data2) {
        Some(OrderData {
            price: data2.price - data1.price,
            size: data1.size.min(data2.size),
        })
    } else {
        None
    }
}

pub fn filter_order_data(data: &[OrderData]) -> Vec<OrderData> {
    let max_size = data.iter().map(|d| d.size).max().unwrap_or_default();
    let threshold = max_size / Decimal::ONE_HUNDRED;
    data.iter().filter(|d| d.size > threshold).cloned().collect()
}

pub fn best_price(data: &[OrderData], is_ask: bool) -> Option<OrderData> {
    let mut best_price = None;
    for item in data {
        match &best_price {
            None => best_price = Some(item.clone()),
            Some(best) => {
                if (is_ask && best.price > item.price) || (!is_ask && best.price < item.price) {
                    best_price = Some(item.clone());
                }
            }
        }
    }
    best_price
}

pub fn best_buy_price(data: &[OrderData], size: Decimal) -> Option<Decimal> {
    data.iter()
        .filter(|item| item.size * item.price >= size)
        .map(|item| item.price)
        .min()
}

pub fn best_sell_price(data: &[OrderData], size: Decimal) -> Option<Decimal> {
    data.iter()
        .filter(|item| item.size * item.price >= size)
        .map(|item| item.price)
        .max()
}

pub fn best_price_for_size(data: &[OrderData], is_ask: bool, size: Decimal) -> Option<Decimal> {
    let mut best_price = None;
    for item in data {
        if item.size * item.price > size {
            match best_price {
                None => best_price = Some(item.price),
                Some(best) => {
                    if (is_ask && best > item.price) || (!is_ask && best < item.price) {
                        best_price = Some(item.price);
                    }
                }
            }
        }
    }
    best_price
}

pub fn weighted_price(data: &[OrderData]) -> Option<OrderData> {
    let (mut sum_price, mut sum_size, mut cnt) = (Decimal::ZERO, Decimal::ZERO, 0);
    for item in data {
        sum_price += item.price * item.size;
        sum_size += item.size;
        cnt += 1;
    }
    if sum_price == Decimal::ZERO {
        None
    } else {
        Some(OrderData {
            price: sum_price / sum_size,
            size: sum_size / Decimal::from(cnt),
        })
    }
}

pub fn save_csv(datetimes: &[DateTime<Utc>], data: &[&Vec<Decimal>], headers: &[&str], filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(filename)?;
    file.write_all(format!("{}\n", headers.join(",")).as_bytes())?;
    for i in 0..datetimes.len() {
        let datetime = datetimes[i].with_nanosecond(0).unwrap().to_rfc3339();
        let line = format!("{datetime},{}\n",
                           data.iter().map(|v| v[i].to_string()).collect::<Vec<_>>().join(","));
        file.write_all(line.as_bytes())?;
    }
    file.sync_all()?;
    Ok(())
}

pub fn handle_delta(deltas: &[OrderData], order_book_data: &mut Vec<OrderData>) {
    for delta_data in deltas {
        if delta_data.size.is_zero() {
            order_book_data.retain(|data| data.price != delta_data.price);
        } else {
            let mut found = false;
            for data in order_book_data.as_mut_slice() {
                if data.price == delta_data.price {
                    data.size = delta_data.size;
                    found = true;
                    break;
                }
            }
            if !found {
                order_book_data.push(delta_data.clone());
            }
        }
    }
}
