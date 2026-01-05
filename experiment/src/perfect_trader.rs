use std::collections::HashMap;
use chrono::{DateTime, Duration, Timelike, Utc};
use log::{error, info};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use model::OrderBook;
use model::utils::{best_buy_price, best_sell_price, calculate_delta_buy, calculate_delta_sell};
use crate::pair_trader::DealInfo;

const AMOUNT: i32 = 500;
const MIN_DEAL_DURATION: i64 = 200;
const MIN_DEAL_PAUSE: i64 = 5000;

#[derive(Clone, Copy, Debug)]
struct Delta {
    pub time: DateTime<Utc>,
    pub buy: Decimal,
    pub sell: Decimal,
}

#[derive(Default)]
pub struct PerfectTrader {
    last_order_books: [Option<OrderBook>; 2],
    deltas: Vec<Delta>,
    prices1: HashMap<DateTime<Utc>, (Decimal, Decimal)>,
    prices2: HashMap<DateTime<Utc>, (Decimal, Decimal)>,
    cur_min: Option<(usize, Delta)>,
    cur_max: Option<(usize, Delta)>,
    last_deal_close_time: DateTime<Utc>,
    prev_time: DateTime<Utc>,
}

impl PerfectTrader {
    pub fn trade(&mut self, order_book: &OrderBook, is_first: bool) -> Option<DealInfo> {
        if self.prev_time.hour() != order_book.timestamp.hour() {
            info!("{}", order_book.timestamp);
        }
        self.prev_time = order_book.timestamp;

        let amount = Decimal::from(AMOUNT);

        self.last_order_books[if is_first { 0 } else { 1 }] = Some(order_book.clone());
        if let (Some(order_book1), Some(order_book2)) =
            (self.last_order_books[0].clone(), self.last_order_books[1].clone()) {

            let sell = best_sell_price(&order_book1.bids, amount);
            let buy = best_buy_price(&order_book1.asks, amount);
            if let (Some(sell), Some(buy)) = (sell, buy) {
                self.prices1.insert(order_book.timestamp, (sell, buy));
            }
            let sell = best_sell_price(&order_book2.bids, amount);
            let buy = best_buy_price(&order_book2.asks, amount);
            if let (Some(sell), Some(buy)) = (sell, buy) {
                self.prices2.insert(order_book.timestamp, (sell, buy));
            }

            if let (Some(delta_buy), Some(delta_sell)) = (
                calculate_delta_buy(&order_book1, &order_book2, amount),
                calculate_delta_sell(&order_book1, &order_book2, amount),
            ) {
                let delta = Delta {
                    time: order_book.timestamp,
                    buy: delta_buy.price,
                    sell: delta_sell.price,
                };
                self.deltas.push(delta);
            }
        }
        if let Some(deal) = self.analyze(order_book.timestamp) {
            info!("{} - {}: {}", deal.open_time, deal.close_time, deal.profit());
            self.last_deal_close_time = deal.close_time;
            self.clean(deal.close_time);
            Some(deal)
        } else {
            None
        }
    }

    fn clean(&mut self, time: DateTime<Utc>) {
        let mut new_deltas = Vec::new();
        let mut new_prices1 = HashMap::new();
        let mut new_prices2 = HashMap::new();
        for delta in &self.deltas {
            if delta.time > time - Duration::minutes(30) {
                new_deltas.push(*delta);
                new_prices1.insert(delta.time, *self.prices1.get(&delta.time).unwrap());
                new_prices2.insert(delta.time, *self.prices2.get(&delta.time).unwrap());
            }
        }
        self.deltas = new_deltas;
        self.prices1 = new_prices1;
        self.prices2 = new_prices2;

        self.cur_min = self.deltas.iter().enumerate()
            .filter(|(_, delta)| delta.time > self.last_deal_close_time + Duration::milliseconds(MIN_DEAL_PAUSE))
            .min_by_key(|(_, delta)| delta.sell)
            .map(|(i, delta)| (i, *delta));
        if let Some((_, cur_min)) = self.cur_min {
            self.cur_max = self.deltas.iter().enumerate()
                .filter(|(_, delta)| delta.time >= cur_min.time + Duration::milliseconds(MIN_DEAL_DURATION))
                .max_by_key(|(_, delta)| delta.buy)
                .map(|(i, delta)| (i, *delta));
        } else {
            self.cur_max = None;
        }
    }
    
    fn analyze(&mut self, cur_time: DateTime<Utc>) -> Option<DealInfo> {
        if self.deltas.len() < 5 {
            return None;
        }

        let last_delta = self.deltas.last().unwrap();
        let last_index = self.deltas.len() - 1;

        let last_is_min = self.cur_min
            .map(|(_, cur_min)| last_delta.sell < cur_min.sell)
            .unwrap_or(true);
        if last_is_min && cur_time > self.last_deal_close_time + Duration::milliseconds(MIN_DEAL_PAUSE) {
            self.cur_min = Some((last_index, *last_delta));
            self.cur_max = None;
        }

        if let Some((_, cur_min)) = &self.cur_min {
            let last_is_max = self.cur_max
                .map(|(_, cur_max)| last_delta.buy > cur_max.buy).unwrap_or(true);
            if last_delta.time >= cur_min.time + Duration::milliseconds(MIN_DEAL_DURATION) && last_is_max {
                self.cur_max = Some((last_index, *last_delta));
            }
        }

        if let (Some((min_index, min_delta)), Some((max_index, max_delta))) = (self.cur_min, self.cur_max) {
            if max_delta.time > cur_time - Duration::minutes(10) {
                return None;
            }
            match self.make_deal(min_index, max_index) {
                Ok(mut deal) => {
                    if deal.profit().is_sign_positive() {
                        if let Some((median_buy, median_sell)) = self.calculate_medians(min_index) {
                            deal.misc = (min_delta.sell / median_sell).to_f64().unwrap_or_default();
                            deal.misc2 = (median_buy / median_sell).to_f64().unwrap_or_default();
                        }
                        Some(deal)
                    } else {
                        None
                    }
                }
                Err(err) => {
                    error!("Failed to make deal: {err}");
                    None
                }
            }
        } else {
            None
        }
    }
    
    fn make_deal(&self, sell_index: usize, buy_index: usize) -> Result<DealInfo, String> {
        let amount = Decimal::from(AMOUNT);
        let time_sell = self.deltas[sell_index].time;
        let (sell1, _) = self.prices1.get(&time_sell).ok_or(format!("No prices1 for {time_sell}"))?;
        let amount1 = amount / sell1;
        let (_, buy2) = self.prices2.get(&time_sell).ok_or(format!("No prices2 for {time_sell}"))?;
        let amount2 = amount / buy2;
        let time_buy = self.deltas[buy_index].time;
        let (_, buy1) = self.prices1.get(&time_buy).ok_or(format!("No prices1 for {time_buy}"))?;
        let (sell2, _) = self.prices2.get(&time_buy).ok_or(format!("No prices2 for {time_buy}"))?;
        Ok(DealInfo {
            open_time: time_sell,
            close_time: time_buy,
            sell_price: [*sell1, *sell2],
            buy_price: [*buy1, *buy2],
            amount: [amount1, amount2],
            misc: 0.0,
            misc2: 0.0,
        })
    }

    fn calculate_medians(&self, index: usize) -> Option<(Decimal, Decimal)> {
        let cur_time = self.deltas[index].time;
        let mut buys = self.deltas.iter()
            .filter(|delta| delta.time > cur_time - Duration::minutes(5))
            .map(|delta| delta.buy)
            .collect::<Vec<Decimal>>();
        if buys.len() < 5 {
            return None;
        }
        buys.sort();
        let median_buy = buys[(buys.len() as f64 * 0.9) as usize];

        let mut sells = self.deltas.iter()
            .filter(|delta| delta.time > cur_time - Duration::minutes(5))
            .map(|delta| delta.sell)
            .collect::<Vec<Decimal>>();
        sells.sort();
        if sells.len() < 5 {
            return None;
        }
        let median_sell = sells[(sells.len() as f64 * 0.1) as usize];

        Some((median_buy, median_sell))
    }
}