use std::collections::HashMap;
use crate::signal_params::{Signal, SignalParams};
use crate::{OrderBookEvent, OrderBookValues};
use chrono::{DateTime, TimeDelta, Utc};

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
    price_time1: DateTime<Utc>,
    price_time2: DateTime<Utc>,
    open_time: DateTime<Utc>,
    direction: DealDirection,
    entry_price1: f64,
    entry_price2: f64,
    close_values1: Option<OrderBookValues>,
    close_values2: Option<OrderBookValues>,
}

impl Deal {
    fn sell1_buy2(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Self {
            price_time1: v1.time,
            price_time2: v2.time,
            open_time: v1.time.max(v2.time),
            direction: DealDirection::Sell1Buy2,

            // Sell GLU6 -> bid
            entry_price1: v1.bid,

            // Buy GLZ6 -> ask
            entry_price2: v2.ask,

            close_values1: None,
            close_values2: None,
        }
    }

    fn buy1_sell2(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Self {
            price_time1: v1.time,
            price_time2: v2.time,
            open_time: v1.time.max(v2.time),
            direction: DealDirection::Buy1Sell2,

            // Buy GLU6 -> ask
            entry_price1: v1.ask,

            // Sell GLZ6 -> bid
            entry_price2: v2.bid,

            close_values1: None,
            close_values2: None,
        }
    }

    fn close_instrument1(&mut self, values: OrderBookValues) {
        if self.close_values1.is_none() {
            self.close_values1 = Some(values);
        }
    }

    fn close_instrument2(&mut self, values: OrderBookValues) {
        if self.close_values2.is_none() {
            self.close_values2 = Some(values);
        }
    }

    fn is_completed(&self) -> bool {
        self.close_values1.is_some() && self.close_values2.is_some()
    }

    fn close(&self, log: bool) -> (DateTime<Utc>, f64, f64) {
        if let Some(v1) = self.close_values1 && let Some (v2) = self.close_values2 {
            match self.direction {
                DealDirection::Sell1Buy2 => {
                    let revenue = self.entry_price1 + v2.bid;
                    let cost = self.entry_price2 + v1.ask;

                    if log {
                        println!(
                            "sell GLU6 {} buy GLZ6 {} on {} -> buy GLU6 {} sell GLZ6 {} on {}, revenue {}, cost {}",
                            self.entry_price1, self.entry_price2, self.open_time,
                            v1.ask, v2.bid, v1.time.max(v2.time),
                            revenue, cost,
                        );
                    }

                    (v1.time.max(v2.time), revenue, cost)
                }

                DealDirection::Buy1Sell2 => {
                    let revenue = self.entry_price2 + v1.bid;
                    let cost = self.entry_price1 + v2.ask;

                    if log {
                        println!(
                            "Buy GLU6 {} sell GLZ6 {} on {} -> sell GLU6 {} buy GLZ6 {} on {}, revenue {}, cost {}",
                            self.entry_price1, self.entry_price2, self.open_time,
                            v1.bid, v2.ask, v2.time.max(v1.time),
                            revenue, cost,
                        );
                    }

                    (v1.time.max(v2.time), revenue, cost)
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
        }
        if let Some(values1) = last1 && let Some(values2) = last2 {
            let cur_signal = params.calc_signal(&values1, &values2);

            if let Some(mut deal) = cur_deal {
                if values1.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument1(values1);
                }
                if values2.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument2(values2);
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
        }
        if let Some(values1) = last1 && let Some(values2) = last2 {
            let cur_signal = params.calc_signal(&values1, &values2);
            println!("Signal: {cur_signal:?}");

            if let Some(mut deal) = cur_deal {
                if values1.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument1(values1);
                }
                if values2.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument2(values2);
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

pub fn run_simulation(events: &[OrderBookEvent], params: &SignalParams, log: bool, threshold: f64) -> SimulationResult {
    let (mut win, mut loss) = (0, 0);
    let (mut total_revenue, mut total_cost) = (0.0, 0.0);
    let mut cur_deal = Option::<Deal>::None;
    let (mut last1, mut last2) = (None, None);
    let mut next_deal_time = Option::<DateTime<Utc>>::None;
    let mut prev_signal = Signal::None;
    for event in events {
        match event {
            OrderBookEvent::Instrument1(values) => last1 = Some(*values),
            OrderBookEvent::Instrument2(values) => last2 = Some(*values),
        }
        if let Some(values1) = last1 && let Some(values2) = last2 {
            let cur_signal = params.calc_signal(&values1, &values2);

            if let Some(mut deal) = cur_deal {
                if values1.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument1(values1);
                }
                if values2.time > deal.open_time + TimeDelta::milliseconds(params.hold_ms as i64) {
                    deal.close_instrument2(values2);
                }
                if deal.is_completed() {
                    let (time, revenue, cost) = deal.close(log);
                    next_deal_time = Some(time + TimeDelta::milliseconds(10));
                    if revenue > cost {win += 1} else {loss += 1};
                    total_revenue += revenue;
                    total_cost += cost;
                    cur_deal = None;
                }
            } else {
                if let Some(next_deal_time) = next_deal_time &&
                    (values1.time < next_deal_time || values2.time < next_deal_time) {
                    continue;
                }

                if cur_signal != prev_signal {
                    cur_deal = match cur_signal {
                        Signal::None => None,
                        Signal::Buy1Sell2(signal) => {
                            if signal > threshold {
                                Some(Deal::buy1_sell2(values1, values2))
                            } else { None }
                        },
                        Signal::Sell1Buy2(signal) => {
                            if signal > threshold {
                                Some(Deal::sell1_buy2(values1, values2))
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
