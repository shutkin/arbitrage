use crate::OrderBookValues;
use crate::signal_params::SignalParams;
use chrono::{DateTime, TimeDelta, Utc};

const COMMISSION_RATIO: f64 = 0.015 / 100.0;

#[derive(Copy, Clone)]
pub enum DealDirection {
    Sell1Buy2, Buy1Sell2,
}

#[derive(Copy, Clone)]
pub struct Deal {
    time: DateTime<Utc>,
    direction: DealDirection,
    price1: f64,
    price2: f64,
}

impl Deal {
    pub fn sell(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Deal {
            time: v1.time,
            direction: DealDirection::Sell1Buy2,
            price1: v1.ask,
            price2: v2.bid,
        }
    }

    pub fn buy(v1: OrderBookValues, v2: OrderBookValues) -> Self {
        Deal {
            time: v1.time,
            direction: DealDirection::Buy1Sell2,
            price1: v1.bid,
            price2: v2.ask,
        }
    }

    pub fn close(&self, v1: OrderBookValues, v2: OrderBookValues) -> (f64, f64, f64) {
        let (mut income, mut outcome, mut commission) = (0.0, 0.0, 0.0);
        match self.direction {
            DealDirection::Sell1Buy2 => {
                income += self.price1 + v2.ask;
                outcome += self.price2 + v1.bid;
                commission += COMMISSION_RATIO * (self.price1 + self.price2 + v1.bid + v2.ask);
            }
            DealDirection::Buy1Sell2 => {
                income += self.price2 + v1.ask;
                outcome += self.price1 + v2.bid;
                commission += COMMISSION_RATIO * (self.price1 + self.price2 + v1.ask + v2.bid);
            }
        }
        (income, outcome, commission)
    }
}

pub struct SimulationResult {
    pub win: u32,
    pub loss: u32,
    pub income: f64,
    pub outcome: f64,
    pub commission: f64,
}

pub fn run_simulation(values1: &[OrderBookValues], values2: &[OrderBookValues], params: &SignalParams) -> SimulationResult {
    let (mut win, mut loss) = (0, 0);
    let (mut total_income, mut total_outcome, mut total_commission) = (0.0, 0.0, 0.0);
    let mut cur_deal = Option::<Deal>::None;
    for i in 32..values1.len() - 32 {
        if let Some(deal) = cur_deal {
            if values1[i].time > deal.time + TimeDelta::milliseconds(params.hold_ms as i64) &&
                values2[i].time > deal.time + TimeDelta::milliseconds(params.hold_ms as i64) {
                let (income, outcome, commission) = deal.close(values1[i], values2[i]);
                if income > outcome {win += 1} else {loss += 1};
                total_income += income;
                total_outcome += outcome;
                total_commission += commission;
                cur_deal = None;
            }
        } else {
            let signal = params.calc_signal(&values1[i], &values2[i]);
            if signal > 0.0 {
                let deal = if values1[i].std_derivative > 0.0 {
                    Deal::sell(values1[i], values2[i])
                } else {
                    Deal::buy(values1[i], values2[i])
                };
                cur_deal = Some(deal);
            }
        }
    }
    SimulationResult {
        win,
        loss,
        income: total_income,
        outcome: total_outcome,
        commission: total_commission,
    }
}
