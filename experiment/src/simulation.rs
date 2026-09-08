use crate::deal::{Deal, DealDirection};
use crate::signal_params::{Signal, SignalParams};
use crate::{COMMISSION_RATIO, MarketEvent, OrderBookValues};
use chrono::{DateTime, TimeDelta, Utc};
use correlation::spearmanr;
use model::Trade;

pub struct SimulationResult {
    pub win: u32,
    pub loss: u32,
    pub income: f64,
    pub outcome: f64,
    pub commission: f64,
}


fn get_close_price_from_values1(deal: &Deal, values: &OrderBookValues) -> f64 {
    match deal.get_direction() {
        DealDirection::Sell1Buy2 => values.bid,
        DealDirection::Buy1Sell2 => values.ask,
    }
}

fn get_close_price_from_values2(deal: &Deal, values: &OrderBookValues) -> f64 {
    match deal.get_direction() {
        DealDirection::Sell1Buy2 => values.ask,
        DealDirection::Buy1Sell2 => values.bid,
    }
}

fn check_close_trade_direction1(deal: &Deal, trade: &Trade) -> bool {
    match deal.get_direction() {
        DealDirection::Sell1Buy2 => trade.direction.map(|dir| dir == 'b').unwrap_or(false),
        DealDirection::Buy1Sell2 => trade.direction.map(|dir| dir == 's').unwrap_or(false),
    }
}

fn check_close_trade_direction2(deal: &Deal, trade: &Trade) -> bool {
    match deal.get_direction() {
        DealDirection::Sell1Buy2 => trade.direction.map(|dir| dir == 's').unwrap_or(false),
        DealDirection::Buy1Sell2 => trade.direction.map(|dir| dir == 'b').unwrap_or(false),
    }
}

pub fn find_order_books_on_horizon(
    events: &[MarketEvent],
    mut index: usize,
    target_time: DateTime<Utc>,
) -> Option<(OrderBookValues, OrderBookValues)> {
    let (mut v1, mut v2) = (None, None);
    while index < events.len() {
        match events[index] {
            MarketEvent::OrderBook1(v) => {
                if v.time >= target_time {
                    v1 = Some(v);
                }
            }
            MarketEvent::OrderBook2(v) => {
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

pub trait DealHandler {
    fn handle_deal(&mut self, events: &[MarketEvent], event_index: usize, deal: Deal);
    fn get_stats(&self) -> Vec<(String, String)>;
    fn init(&mut self, variant: u8);
}

pub fn run_stats(events: &[MarketEvent], params: &SignalParams, deal_handler: &mut dyn DealHandler) -> Vec<(String, String)> {
    let (mut last_order_book1, mut last_order_book2) = (None, None);
    let mut prev_signal = Signal::None;

    for (index, event) in events.iter().enumerate() {
        match event {
            MarketEvent::OrderBook1(values) => last_order_book1 = Some(*values),
            MarketEvent::OrderBook2(values) => last_order_book2 = Some(*values),
            MarketEvent::Deal1(_) => {},
            MarketEvent::Deal2(_) => {},
        }

        if let Some(values1) = &last_order_book1 && let Some(values2) = &last_order_book2 {
            let cur_signal = params.signal(values1, values2);

            if cur_signal != prev_signal &&
                let Some(deal) = match cur_signal {
                    Signal::Buy1Sell2 => Some(Deal::buy1_sell2(values1, values2)),
                    Signal::Sell1Buy2 => Some(Deal::sell1_buy2(values1, values2)),
                    _ => None,
                } { deal_handler.handle_deal(events, index, deal); }

            prev_signal = cur_signal;
        }
    }
    deal_handler.get_stats()
}

pub fn run_simulation(events: &[MarketEvent], params: &SignalParams) -> SimulationResult {
    let (mut win, mut loss) = (0, 0);
    let (mut total_revenue, mut total_cost) = (0.0, 0.0);
    let mut cur_deal = Option::<Deal>::None;
    let (mut last_order_book1, mut last_order_book2) = (None, None);
    let mut prev_signal = Signal::None;

    for (i, event) in events.iter().enumerate() {
        match event {
            MarketEvent::OrderBook1(values) => last_order_book1 = Some(*values),
            MarketEvent::OrderBook2(values) => last_order_book2 = Some(*values),
            _ => {},
        }

        if let Some(deal) = cur_deal.as_mut() {
            if let Some(v) = last_order_book1 &&
                v.time > deal.get_open_time() + TimeDelta::milliseconds(params.hold_ms as i64) {
                let price = match deal.get_direction() {
                    DealDirection::Sell1Buy2 => v.bid,
                    DealDirection::Buy1Sell2 => v.ask,
                };
                deal.close_instrument1(price, v.time)
            }
            if let Some(v) = last_order_book2 &&
                v.time > deal.get_open_time() + TimeDelta::milliseconds(params.hold_ms as i64) {
                let price = match deal.get_direction() {
                    DealDirection::Sell1Buy2 => v.ask,
                    DealDirection::Buy1Sell2 => v.bid,
                };
                deal.close_instrument2(price, v.time)
            }
            if deal.is_completed() {
                let (time, revenue, cost) = deal.close(false);
                if revenue > cost {win += 1} else {loss += 1};
                total_revenue += revenue;
                total_cost += cost;
                cur_deal = None;
            }
        }

        if let Some(values1) = &last_order_book1 && let Some(values2) = &last_order_book2 {
            let cur_signal = params.signal(values1, values2);

            if cur_deal.is_none() &&
                cur_signal != prev_signal {
                cur_deal = match cur_signal {
                    Signal::None => None,
                    Signal::Buy1Sell2 => {
                        Some(Deal::buy1_sell2(values1, values2))
                    },
                    Signal::Sell1Buy2 => {
                        Some(Deal::sell1_buy2(values1, values2))
                    },
                };
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

pub fn run_simulation_on_trades(events: &[MarketEvent], params: &SignalParams, latency: u32, log: bool) -> SimulationResult {
    let (mut win, mut loss) = (0, 0);
    let (mut total_revenue, mut total_cost) = (0.0, 0.0);
    let mut cur_deal = Option::<Deal>::None;
    let (mut last_order_book1, mut last_order_book2) = (None, None);
    let (mut last_deal1, mut last_deal2) = (None, None);
    let mut next_deal_time = Option::<DateTime<Utc>>::None;
    let mut prev_signal = Signal::None;
    for (i, event) in events.iter().enumerate() {
        match event {
            MarketEvent::OrderBook1(values) => last_order_book1 = Some(*values),
            MarketEvent::OrderBook2(values) => last_order_book2 = Some(*values),
            MarketEvent::Deal1(trade) => last_deal1 = Some(*trade),
            MarketEvent::Deal2(trade) => last_deal2 = Some(*trade),
        }

        if let Some(mut deal) = cur_deal {
            if let Some(trade) = last_deal1 &&
                check_close_trade_direction1(&deal, &trade) &&
                trade.created > deal.get_open_time() + TimeDelta::milliseconds(params.hold_ms as i64) &&
                let Some(price) = trade.price {
                deal.close_instrument1(price.as_f64(), trade.created)
            }
            if let Some(trade) = last_deal2 &&
                check_close_trade_direction2(&deal, &trade) &&
                trade.created > deal.get_open_time() + TimeDelta::milliseconds(params.hold_ms as i64) &&
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

        if let Some(values1) = &last_order_book1 && let Some(values2) = &last_order_book2 {
            let cur_signal = params.signal(&values1, &values2);

            if cur_deal.is_none() {
                if let Some(next_deal_time) = next_deal_time &&
                    (values1.time < next_deal_time || values2.time < next_deal_time) {
                    continue;
                }

                if cur_signal != prev_signal {
                    cur_deal = match cur_signal {
                        Signal::None => None,
                        Signal::Buy1Sell2 => {
                            if latency == 0 {
                                Some(Deal::buy1_sell2(values1, values2))
                            } else if let Some((v1, v2)) =
                                find_order_books_on_horizon(events, i, values1.time.max(values2.time) + TimeDelta::milliseconds(latency as i64)) {
                                Some(Deal::buy1_sell2(&v1, &v2))
                            } else { None }
                        },
                        Signal::Sell1Buy2 => {
                            if latency == 0 {
                                Some(Deal::sell1_buy2(values1, values2))
                            } else if let Some((v1, v2)) =
                                find_order_books_on_horizon(events, i, values1.time.max(values2.time) + TimeDelta::milliseconds(latency as i64)) {
                                Some(Deal::sell1_buy2(&v1, &v2))
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
