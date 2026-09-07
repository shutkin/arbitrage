use std::collections::HashMap;
use crate::signal_params::{Signal, SignalParams};
use crate::{OrderBookEvent, OrderBookValues};
use chrono::{DateTime, TimeDelta, Utc};
use model::Trade;

const COMMISSION_RATIO: f64 = 0.015 / 100.0;

pub struct SimulationResult {
    pub win: u32,
    pub loss: u32,
    pub income: f64,
    pub outcome: f64,
    pub commission: f64,
}

#[derive(Copy, Clone)]
pub enum DealDirection {
    Sell1Buy2, Buy1Sell2,
}

#[derive(Copy, Clone)]
pub struct Deal {
    direction: DealDirection,
    open_time: DateTime<Utc>,
    entry_price1: f64,
    entry_price2: f64,
    close_price1: Option<f64>,
    close_price2: Option<f64>,
    close_time1: Option<DateTime<Utc>>,
    close_time2: Option<DateTime<Utc>>,
}

impl Deal {
    fn sell1_buy2(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Self {
            open_time: v1.time.max(v2.time),
            direction: DealDirection::Sell1Buy2,

            // Sell GLU6 -> bid
            entry_price1: v1.bid,

            // Buy GLZ6 -> ask
            entry_price2: v2.ask,

            close_price1: None,
            close_price2: None,
            close_time1: None,
            close_time2: None,
        }
    }

    fn buy1_sell2(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Self {
            open_time: v1.time.max(v2.time),
            direction: DealDirection::Buy1Sell2,

            // Buy GLU6 -> ask
            entry_price1: v1.ask,

            // Sell GLZ6 -> bid
            entry_price2: v2.bid,

            close_price1: None,
            close_price2: None,
            close_time1: None,
            close_time2: None,
        }
    }

    fn close_instrument1(&mut self, price: f64, time: DateTime<Utc>) {
        if self.close_price1.is_none() {
            self.close_price1 = Some(price);
            self.close_time1 = Some(time);
        }
    }

    fn close_instrument2(&mut self, price: f64, time: DateTime<Utc>) {
        if self.close_price2.is_none() {
            self.close_price2 = Some(price);
            self.close_time2 = Some(time);
        }
    }

    fn is_completed(&self) -> bool {
        self.close_price1.is_some() && self.close_price2.is_some()
    }

    fn close(&self, log: bool) -> (DateTime<Utc>, f64, f64) {
        if let Some(p1) = self.close_price1 && let Some (p2) = self.close_price2 &&
            let Some(close_time1) = self.close_time1 && let Some(close_time2) = self.close_time2 {
            match self.direction {
                DealDirection::Sell1Buy2 => {
                    let revenue = self.entry_price1 + p2; // bid
                    let cost = self.entry_price2 + p1; // ask

                    if log {
                        println!(
                            "sell GLU6 @ {} buy GLZ6 @ {} at {} -> buy GLU6 @ {} at {} sell GLZ6 {} at {}, revenue {}, cost {}",
                            self.entry_price1, self.entry_price2, self.open_time,
                            p1, close_time1, p2, close_time2,
                            revenue, cost,
                        );
                    }

                    (close_time1.max(close_time2), revenue, cost)
                }

                DealDirection::Buy1Sell2 => {
                    let revenue = self.entry_price2 + p1; // bid
                    let cost = self.entry_price1 + p2; // ask

                    if log {
                        println!(
                            "Buy GLU6 @ {} sell GLZ6 @ {} at {} -> sell GLU6 @ {} at {} buy GLZ6 @ {} at {}, revenue {}, cost {}",
                            self.entry_price1, self.entry_price2, self.open_time,
                            p1, close_time1, p2, close_time2,
                            revenue, cost,
                        );
                    }

                    (close_time1.max(close_time2), revenue, cost)
                }
            }
        } else {
            panic!("Close unfinished deal");
        }
    }
}

pub fn signal_strength_map(events: &[OrderBookEvent], params: &SignalParams) {
    let mut cur_deal = Option::<Deal>::None;
    let mut cur_deal_strength = 0.0;
    let (mut last1, mut last2) = (None, None);
    let mut prev_signal = Signal::None;
    let mut signal_profit = Vec::new();
    for event in events {
        match event {
            OrderBookEvent::Instrument1(values) => {
                last1 = Some(*values)
            },
            OrderBookEvent::Instrument2(values) => {
                last2 = Some(*values)
            },
            _ => {},
        }
        if let Some(values1) = last1 && let Some(values2) = last2 {
            let cur_signal = params.calc_signal(&values1, &values2);

            if let Some(mut deal) = cur_deal {
                if values1.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument1(get_close_price_from_values1(&deal, &values1), values1.time);
                }
                if values2.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument2(get_close_price_from_values2(&deal, &values2), values2.time);
                }
                if deal.is_completed() {
                    let (_, revenue, cost) = deal.close(false);
                    signal_profit.push((cur_deal_strength, revenue - cost));
                    cur_deal = None;
                }
            } else {
                if cur_signal != prev_signal {
                    cur_deal = match cur_signal {
                        Signal::None => None,
                        Signal::Buy1Sell2(signal_strength) => {
                            cur_deal_strength = signal_strength;
                            Some(Deal::buy1_sell2(values1, values2))
                        },
                        Signal::Sell1Buy2(signal_strength) => {
                            cur_deal_strength = signal_strength;
                            Some(Deal::sell1_buy2(values1, values2))
                        },
                    };
                }
            }

            prev_signal = cur_signal;
        }
    }
    let max_signal = signal_profit.iter().map(|(signal, _)| *signal).reduce(f64::max).unwrap_or(0.0);
    println!("Max signal {max_signal}");
    let mut backets = HashMap::new();
    for (signal, profit) in signal_profit {
        let backet_index = (10.0 * signal / max_signal) as usize;
        backets.entry(backet_index).or_insert(Vec::new()).push((signal, profit));
    }
    for i in 0..backets.len() {
        if let Some(profits) = backets.get(&i) {
            let avg_signal = profits.iter().map(|(signal, _)| *signal).sum::<f64>() / profits.len() as f64;
            let avg_profit = profits.iter().map(|(_, profit)| *profit).sum::<f64>() / profits.len() as f64;
            println!("~{avg_signal}: {} deals with average profit {avg_profit}", profits.len());
        }
    }
}

pub fn log_simulation(events: &[OrderBookEvent], params: &SignalParams) {
    let mut signal_changes = 0;
    let mut cur_deal = Option::<Deal>::None;
    let (mut last1, mut last2) = (None, None);
    let mut prev_signal = Signal::None;
    for event in events {
        match event {
            OrderBookEvent::Instrument1(values) => {
                println!("Instrument1 event: {} - ask {}, bid {}", values.time, values.ask, values.bid);
                last1 = Some(*values)
            },
            OrderBookEvent::Instrument2(values) => {
                println!("Instrument2 event: {} - ask {}, bid {}", values.time, values.ask, values.bid);
                last2 = Some(*values)
            },
            _ => {},
        }
        if let Some(values1) = last1 && let Some(values2) = last2 {
            let cur_signal = params.calc_signal(&values1, &values2);
            println!("Signal: {cur_signal:?}");

            if let Some(mut deal) = cur_deal {
                if values1.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument1(get_close_price_from_values1(&deal, &values1), values1.time);
                }
                if values2.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument2(get_close_price_from_values2(&deal, &values2), values2.time);
                }
                if deal.is_completed() {
                    let _ = deal.close(true);
                    cur_deal = None;
                }
            } else {
                if cur_signal != prev_signal {
                    signal_changes += 1;
                    cur_deal = match cur_signal {
                        Signal::None => None,
                        Signal::Buy1Sell2(_) => {
                            println!("Buy 1 sell 2");
                            Some(Deal::buy1_sell2(values1, values2))
                        },
                        Signal::Sell1Buy2(_) => {
                            println!("Buy 1 sell 2");
                            Some(Deal::sell1_buy2(values1, values2))
                        },
                    };
                }
            }

            prev_signal = cur_signal;
        }
    }
    println!("Signal changes: {signal_changes}");
}

fn get_close_price_from_values1(deal: &Deal, values: &OrderBookValues) -> f64 {
    match deal.direction {
        DealDirection::Sell1Buy2 => values.bid,
        DealDirection::Buy1Sell2 => values.ask,
    }
}

fn get_close_price_from_values2(deal: &Deal, values: &OrderBookValues) -> f64 {
    match deal.direction {
        DealDirection::Sell1Buy2 => values.ask,
        DealDirection::Buy1Sell2 => values.bid,
    }
}

fn check_close_trade_direction1(deal: &Deal, trade: &Trade) -> bool {
    match deal.direction {
        DealDirection::Sell1Buy2 => trade.direction.map(|dir| dir == 'b').unwrap_or(false),
        DealDirection::Buy1Sell2 => trade.direction.map(|dir| dir == 's').unwrap_or(false),
    }
}

fn check_close_trade_direction2(deal: &Deal, trade: &Trade) -> bool {
    match deal.direction {
        DealDirection::Sell1Buy2 => trade.direction.map(|dir| dir == 's').unwrap_or(false),
        DealDirection::Buy1Sell2 => trade.direction.map(|dir| dir == 'b').unwrap_or(false),
    }
}

fn find_order_books_with_latency(
    events: &[OrderBookEvent],
    mut index: usize,
    v1: &OrderBookValues,
    v2: &OrderBookValues,
    latency: u16
) -> Option<(OrderBookValues, OrderBookValues)> {
    if latency == 0 {
        return Some((*v1, *v2));
    }

    let target_time = v1.time.max(v2.time) + TimeDelta::milliseconds(latency as i64);
    let (mut v1, mut v2) = (None, None);
    while index < events.len() {
        match events[index] {
            OrderBookEvent::Instrument1(v) => {
                if v.time >= target_time {
                    v1 = Some(v);
                }
            }
            OrderBookEvent::Instrument2(v) => {
                if v.time >= target_time {
                    v2 = Some(v);
                }
            }
            _ => {}
        }
        if let Some(v1) = v1 && let Some(v2) = v2 {
            return Some((v1, v2));
        }
        index += 1;
    }
    None
}

pub fn run_simulation(events: &[OrderBookEvent], params: &SignalParams, signal_threshold: f64, latency: u16, log: bool) -> SimulationResult {
    let (mut win, mut loss) = (0, 0);
    let (mut total_revenue, mut total_cost) = (0.0, 0.0);
    let mut cur_deal = Option::<Deal>::None;
    let (mut last_order_book1, mut last_order_book2) = (None, None);
    let (mut last_deal1, mut last_deal2) = (None, None);
    let mut next_deal_time = Option::<DateTime<Utc>>::None;
    let mut prev_signal = Signal::None;
    for (i, event) in events.iter().enumerate() {
        match event {
            OrderBookEvent::Instrument1(values) => last_order_book1 = Some(*values),
            OrderBookEvent::Instrument2(values) => last_order_book2 = Some(*values),
            OrderBookEvent::Deal1(trade) => last_deal1 = Some(*trade),
            OrderBookEvent::Deal2(trade) => last_deal2 = Some(*trade),
        }

        if let Some(mut deal) = cur_deal {
            if let Some(trade) = last_deal1 &&
                check_close_trade_direction1(&deal, &trade) &&
                trade.created > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) &&
                let Some(price) = trade.price {
                deal.close_instrument1(price.as_f64(), trade.created)
            }
            if let Some(trade) = last_deal2 &&
                check_close_trade_direction2(&deal, &trade) &&
                trade.created > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) &&
                let Some(price) = trade.price {
                deal.close_instrument2(price.as_f64(), trade.created)
            }
            if deal.is_completed() {
                let (time, revenue, cost) = deal.close(log);
                next_deal_time = Some(time + TimeDelta::milliseconds(10));
                if revenue > cost {win += 1} else {loss += 1};
                total_revenue += revenue;
                total_cost += cost;
                cur_deal = None;
            }
        }

        if let Some(values1) = last_order_book1 && let Some(values2) = last_order_book2 {
            let cur_signal = params.calc_signal(&values1, &values2);

            if cur_deal.is_none() {
                if let Some(next_deal_time) = next_deal_time &&
                    (values1.time < next_deal_time || values2.time < next_deal_time) {
                    continue;
                }

                if cur_signal != prev_signal {
                    cur_deal = match cur_signal {
                        Signal::None => None,
                        Signal::Buy1Sell2(signal) => {
                            if signal > signal_threshold &&
                                let Some((v1, v2)) = find_order_books_with_latency(events, i, &values1, &values2, latency) {
                                Some(Deal::buy1_sell2(v1, v2))
                            } else { None }
                        },
                        Signal::Sell1Buy2(signal) => {
                            if signal > signal_threshold &&
                                let Some((v1, v2)) = find_order_books_with_latency(events, i, &values1, &values2, latency) {
                                Some(Deal::sell1_buy2(v1, v2))
                            } else { None }
                        },
                    };
                }
            }

            prev_signal = cur_signal;
        }
    }
    SimulationResult {
        win,
        loss,
        income: total_revenue,
        outcome: total_cost,
        commission: (total_revenue + total_cost) * COMMISSION_RATIO,
    }
}
