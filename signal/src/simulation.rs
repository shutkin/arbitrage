use chrono::{DateTime, TimeDelta, Utc};
use crate::orderbook_values::{Leg, OrderBookValues};
use crate::signal_calculator::CalculatorsPair;
use crate::TradeSignal;

#[derive(Copy, Clone)]
struct SimDeal {
    signal: TradeSignal,
    close_time: DateTime<Utc>,
    open_price1: f64,
    open_price2: f64,
    close_price1: Option<f64>,
    close_price2: Option<f64>,
}

pub fn run_simulation(values: &[OrderBookValues], calculator: &CalculatorsPair, hold_time_ms: u16) -> (u32, f64) {
    let (mut v1, mut v2) = (None, None);
    let mut active_deal = Option::<SimDeal>::None;
    let (mut total_revenue, mut total_cost, mut total_commission) = (0.0, 0.0, 0.0);
    let mut deals_count = 0;

    for value in values {
        match value.leg {
            Leg::First => v1 = Some(value),
            Leg::Second => v2 = Some(value),
        }
        
        if let Some(v1) = v1 && let Some(v2) = v2 {
            if let Some(deal) = active_deal.as_mut() {

                if deal.close_price1.is_none() && v1.time >= deal.close_time {
                    deal.close_price1 = Some(match deal.signal {
                        TradeSignal::Sell1Buy2(_) => v1.ask,
                        TradeSignal::Buy1Sell2(_) => v1.bid,
                        TradeSignal::None => unreachable!(),
                    });
                }
                if deal.close_price2.is_none() && v2.time >= deal.close_time {
                    deal.close_price2 = Some(match deal.signal {
                        TradeSignal::Sell1Buy2(_) => v2.bid,
                        TradeSignal::Buy1Sell2(_) => v2.ask,
                        TradeSignal::None => unreachable!(),
                    });
                }

                if let Some(close_price1) = deal.close_price1 && let Some(close_price2) = deal.close_price2 {
                    let revenue = match deal.signal {
                        TradeSignal::Sell1Buy2(_) => deal.open_price1 + close_price2,
                        TradeSignal::Buy1Sell2(_) => deal.open_price2 + close_price1,
                        TradeSignal::None => unreachable!(),
                    };
                    let cost = match deal.signal {
                        TradeSignal::Sell1Buy2(_) => deal.open_price2 + close_price1,
                        TradeSignal::Buy1Sell2(_) => deal.open_price1 + close_price2,
                        TradeSignal::None => unreachable!(),
                    };
                    active_deal = None;

                    total_revenue += revenue;
                    total_cost += cost;
                    total_commission += 5.0;
                    deals_count += 1;
                }

            } else {
                let signal = calculator.calculate(v1, v2, hold_time_ms);
                active_deal = match signal {
                    TradeSignal::Sell1Buy2(hold) => {
                        let cur_time = v1.time.max(v2.time);
                        Some(SimDeal {
                            signal,
                            close_time: cur_time + TimeDelta::milliseconds(hold as i64),
                            open_price1: v1.bid,
                            open_price2: v2.ask,
                            close_price1: None,
                            close_price2: None,
                        })
                    }
                    TradeSignal::Buy1Sell2(hold) => {
                        let cur_time = v1.time.max(v2.time);
                        Some(SimDeal {
                            signal,
                            close_time: cur_time + TimeDelta::milliseconds(hold as i64),
                            open_price1: v1.ask,
                            open_price2: v2.bid,
                            close_price1: None,
                            close_price2: None,
                        })
                    },
                    TradeSignal::None => None,
                };
            }
        }
    }

    (deals_count, total_revenue - total_cost - total_commission)
}
