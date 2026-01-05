use chrono::{DateTime, Duration, Utc};
use log::{debug, info};
use model::OrderBook;
use model::utils::{best_buy_price, best_sell_price, calculate_delta_buy, calculate_delta_sell};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
struct Delta {
    pub time: DateTime<Utc>,
    pub buy: f64,
    pub sell: f64,
}

#[derive(Clone)]
pub struct PairTraderConfig {
    pub median_interval_minutes: u16,
    pub delta_to_median_ratio: f64,
    pub medians_ratio: f64,
    pub median_lifetime_seconds: u16,
    pub min_deal_interval_millis: u16,
    pub calc_median: bool,
    pub deal_amount: Decimal,
}

#[derive(Clone)]
pub struct PairTrader {
    config: PairTraderConfig,
    deltas: Vec<Delta>,
    cur_medians: Option<Delta>,
    last_order_books: [Option<OrderBook>; 2],
    cur_deal: Option<DealInfo>,
    last_deal_time: DateTime<Utc>,
}

#[derive(Clone, Copy)]
pub struct DealInfo {
    pub open_time: DateTime<Utc>,
    pub close_time: DateTime<Utc>,
    pub amount: [Decimal; 2],
    pub buy_price: [Decimal; 2],
    pub sell_price: [Decimal; 2],
    pub misc: f64,
    pub misc2: f64,
}

impl DealInfo {
    pub fn profit(&self) -> Decimal {
        let mut profit = Decimal::ZERO;
        let mut commission = Decimal::ZERO;
        for i in 0..2 {
            profit += (self.sell_price[i] - self.buy_price[i]) * self.amount[i];
            commission += (self.buy_price[i] + self.sell_price[i]) * self.amount[i] * Decimal::from(40) / Decimal::from(1000 * 100);
        }
        profit - commission
    }
}

impl PairTrader {
    pub fn new(config: PairTraderConfig) -> Self {
        Self {
            config,
            deltas: Vec::new(),
            last_order_books: [None, None],
            cur_deal: None,
            last_deal_time: Default::default(),
            cur_medians: None,
        }
    }

    pub fn trade(&mut self, order_book: &OrderBook, is_first: bool) -> Option<DealInfo> {
        if let Some(cur_median_time) = self.cur_medians.map(|median| median.time)
            && cur_median_time < order_book.timestamp - Duration::seconds(self.config.median_lifetime_seconds as i64) {
            self.cur_medians = None;
        }

        self.last_order_books[if is_first { 0 } else { 1 }] = Some(order_book.clone());
        if let (Some(order_book1), Some(order_book2)) = (
            self.last_order_books[0].clone(),
            self.last_order_books[1].clone(),
        ) {
            if let (Some(delta_buy), Some(delta_sell)) = (
                calculate_delta_buy(&order_book1, &order_book2, self.config.deal_amount),
                calculate_delta_sell(&order_book1, &order_book2, self.config.deal_amount),
            ) {
                let delta = Delta {
                    time: order_book.timestamp,
                    buy: delta_buy.price.to_f64().unwrap(),
                    sell: delta_sell.price.to_f64().unwrap(),
                };

                if order_book.timestamp > self.last_deal_time + Duration::milliseconds(self.config.min_deal_interval_millis as i64) {
                    let medians = if let Some(cur_medians) = self.cur_medians {
                        Some(cur_medians)
                    } else {
                        if self.config.calc_median {
                            self.calculate_medians(order_book.timestamp)
                        } else {
                            self.calculate_averages(order_book.timestamp)
                        }.map(|(median_buy, median_sell)| {
                                let medians = Delta {
                                    time: order_book.timestamp,
                                    buy: median_buy,
                                    sell: median_sell,
                                };
                                self.cur_medians = Some(medians);
                                medians
                            },
                        )
                    };
                    if let Some(medians) = medians {
                        if let Some(deal) = self.cur_deal {
                            return self.close_deal(deal, &order_book1, &order_book2, delta, medians);
                        } else {
                            self.init_deal(&order_book1, &order_book2, delta, medians);
                        }
                    }
                }

                self.deltas.push(delta);
                self.clean_deltas(order_book.timestamp);
            } else {
                debug!("{}: can't calculate deltas", order_book.timestamp);
            }
        }
        None
    }

    fn init_deal(&mut self, order_book1: &OrderBook, order_book2: &OrderBook, cur_delta: Delta, medians: Delta) {
        let delta_to_median_ratio = cur_delta.sell / medians.sell;
        let medians_ratio = medians.buy / medians.sell;
        if delta_to_median_ratio < self.config.delta_to_median_ratio && medians_ratio > self.config.medians_ratio {
            info!("{}: current sell delta {}, median {}", cur_delta.time, cur_delta.sell, medians.sell);
            if let (Some(sell1_price), Some(buy2_price)) = (
                best_sell_price(&order_book1.bids, self.config.deal_amount),
                best_buy_price(&order_book2.asks, self.config.deal_amount),
            ) {
                let (amount1, amount2) = (self.config.deal_amount / sell1_price, self.config.deal_amount / buy2_price);
                self.cur_deal = Some(DealInfo {
                    open_time: cur_delta.time,
                    close_time: Default::default(),
                    amount: [amount1, amount2],
                    buy_price: [Decimal::ZERO, buy2_price],
                    sell_price: [sell1_price, Decimal::ZERO],
                    misc: cur_delta.sell / medians.sell,
                    misc2: medians.buy / medians.sell,
                });
                info!(
                    "{}: sell 1 {} for {sell1_price}, buy 2 {} for {buy2_price}",
                    cur_delta.time, amount1, amount2
                );

                self.last_deal_time = cur_delta.time;
            }
        }
    }

    fn close_deal(&mut self, mut deal: DealInfo, order_book1: &OrderBook, order_book2: &OrderBook, cur_delta: Delta, medians: Delta) -> Option<DealInfo> {
        //if cur_delta.buy > medians.buy || cur_delta.time > deal.open_time + Duration::hours(6) {
            if let (Some(buy1_price), Some(sell2_price)) = (
                best_buy_price(&order_book1.asks, self.config.deal_amount),
                best_sell_price(&order_book2.bids, self.config.deal_amount),
            ) {
                deal.close_time = cur_delta.time;
                deal.buy_price[0] = buy1_price;
                deal.sell_price[1] = sell2_price;
                info!("{} {}->{}={} {}->{}={} {}", cur_delta.time,
                    deal.buy_price[0], deal.sell_price[0], deal.sell_price[0] - deal.buy_price[0],
                    deal.buy_price[1], deal.sell_price[1], deal.sell_price[1] - deal.buy_price[1],
                    deal.sell_price[0] - deal.buy_price[0] + deal.sell_price[1] - deal.buy_price[1]);
                if deal.profit().is_sign_positive() || cur_delta.time > deal.open_time + Duration::hours(1) {
                    info!(
                        "{}: buy 1 {} for {buy1_price}, sell 2 {} for {sell2_price}",
                        cur_delta.time, deal.amount[0], deal.amount[1]
                    );
                    info!("Deal profit: {}", deal.profit());

                    self.cur_deal = None;
                    self.last_deal_time = cur_delta.time;
                    return Some(deal);
                }
            }
        //}
        None
    }

    fn calculate_averages(&self, cur_time: DateTime<Utc>) -> Option<(f64, f64)> {
        if self.deltas.is_empty()
            || cur_time - Duration::minutes(self.config.median_interval_minutes as i64)
            < self.deltas[0].time - Duration::minutes(5)
        {
            None
        } else {
            let buy_avg = self.deltas.iter()
                .map(|delta| delta.buy)
                .sum::<f64>() / self.deltas.len() as f64;

            let sell_avg = self.deltas.iter()
                .map(|delta| delta.sell)
                .sum::<f64>() / self.deltas.len() as f64;

            Some((buy_avg, sell_avg))
        }
    }

    fn calculate_medians(&self, cur_time: DateTime<Utc>) -> Option<(f64, f64)> {
        if self.deltas.is_empty()
            || cur_time - Duration::minutes(self.config.median_interval_minutes as i64)
                < self.deltas[0].time - Duration::minutes(5)
        {
            None
        } else {
            let mut buys = self.deltas.iter()
                .map(|delta| delta.buy)
                .collect::<Vec<f64>>();
            buys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let median_buy = buys[(buys.len() as f64 * 0.9) as usize];

            let mut sells = self.deltas.iter()
                .map(|delta| delta.sell)
                .collect::<Vec<f64>>();
            sells.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let median_sell = sells[(sells.len() as f64 * 0.1) as usize];

            Some((median_buy, median_sell))
        }
    }

    fn clean_deltas(&mut self, cur_time: DateTime<Utc>) {
        let threshold = cur_time - Duration::minutes(self.config.median_interval_minutes as i64);
        while !self.deltas.is_empty() && self.deltas[0].time < threshold {
            self.deltas.remove(0);
        }
    }
}
