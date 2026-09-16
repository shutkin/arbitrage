mod orderbook_values;
mod signal_calculator;
mod optimizer;
mod simulation;

use crate::optimizer::calibrate_signal_calculator;
use crate::orderbook_values::{Leg, OrderBookValues, WindowValuesCalculator};
use crate::signal_calculator::CalculatorsPair;
use chrono::TimeDelta;
use log::{debug, error};
use model::OrderBook;
use std::thread;

const TRAIN_VALUES_SIZE: usize = 2 * 2 * 30000;

const STD_DIAPASON_SECONDS: u16 = 1050;
const HOLD_TIME_MS: u16 = 1100;

#[derive(Copy, Clone)]
pub enum TradeSignal {
    None,
    Sell1Buy2(u16),
    Buy1Sell2(u16),
}

pub struct Signal {
    ticker1: String,
    ticker2: String,

    std_diapason_s: u16,
    hold_time_ms: u16,

    values1_calculator: WindowValuesCalculator,
    values2_calculator: WindowValuesCalculator,

    calculator: Option<CalculatorsPair>,

    buffer: Vec<OrderBookValues>,
}

impl Signal {
    pub fn new(ticker1: &str, ticker2: &str) -> Self {
        Self {
            ticker1: ticker1.to_string(),
            ticker2: ticker2.to_string(),
            std_diapason_s: STD_DIAPASON_SECONDS,
            hold_time_ms: HOLD_TIME_MS,

            values1_calculator: WindowValuesCalculator::new(Leg::First),
            values2_calculator: WindowValuesCalculator::new(Leg::Second),

            calculator: None,

            buffer: Vec::new(),
        }
    }

    fn get_last_values(&self) -> Option<(&OrderBookValues, &OrderBookValues)> {
        let (mut v1, mut v2) = (None, None);
        let mut i = self.buffer.len() - 1;
        while i > 0 && (v1.is_none() || v2.is_none()) {
            match self.buffer[i].leg {
                Leg::First => if v1.is_none() { v1 = Some(&self.buffer[i]) },
                Leg::Second => if v2.is_none() { v2 = Some(&self.buffer[i]) },
            }
            i -= 1;
        }

        if let Some(v1) = v1 && let Some(v2) = v2 {
            Some((v1, v2))
        } else {
            None
        }
    }

    pub fn process(&mut self, ticker: &str, order_book: &OrderBook) -> TradeSignal {
        let v = if self.ticker1 == ticker {
            self.values1_calculator.calculate(order_book, self.std_diapason_s)
        } else if self.ticker2 == ticker {
            self.values2_calculator.calculate(order_book, self.std_diapason_s)
        } else {
            return TradeSignal::None;
        };
        self.buffer.push(v);

        if let Some(calculator) = &self.calculator
            && let Some((v1, v2)) = self.get_last_values() {
            calculator.calculate(v1, v2, self.hold_time_ms)
        } else {
            TradeSignal::None
        }
    }
    
    pub fn calibrate(&mut self) {
        if self.buffer.len() < TRAIN_VALUES_SIZE {
            return;
        }

        let last_time = self.buffer[self.buffer.len() - 1].time;
        if self.calculator.as_ref().map(|calc| last_time - calc.get_created_on() > TimeDelta::minutes(5)).unwrap_or(true) {
            self.shrink_buffer();

            let hold_time = self.hold_time_ms;

            let buf_clone = self.buffer.clone();
            let thread_up = thread::spawn(move || {
                match calibrate_signal_calculator(&buf_clone, true, hold_time) {
                    Ok(calc) => Some(calc),
                    Err(err) => {
                        error!("Failed to optimize UP calculator: {err}");
                        None
                    }
                }
            });

            let buf_clone = self.buffer.clone();
            let thread_down = thread::spawn(move || {
                match calibrate_signal_calculator(&buf_clone, false, hold_time) {
                    Ok(calc) => Some(calc),
                    Err(err) => {
                        error!("Failed to optimize DOWN calculator: {err}");
                        None
                    }
                }
            });

            if let Some(up) = thread_up.join().ok().flatten()
                && let Some(down) = thread_down.join().ok().flatten() {
                let last_time = self.buffer[self.buffer.len() - 1].time;
                let calc = CalculatorsPair::new(up, down, last_time);
                debug!("{calc:?}");
                self.calculator = Some(calc);
            }
        }
    }

    fn shrink_buffer(&mut self) {
        if self.buffer.len() > TRAIN_VALUES_SIZE + 1 {
            let to_delete = self.buffer.len() - TRAIN_VALUES_SIZE - 1;
            self.buffer.drain(0..to_delete);
        }
    }
}