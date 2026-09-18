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

const ORDER_BOOKS_PER_HOUR: usize = 24000;
const TRAIN_DATA_MINUTES: u16 = 60;
const TRAIN_INTERVAL_MINUTES: u16 = 5;

const STD_DIAPASON_SECONDS: u16 = 1050;
const HOLD_TIME_MS: u16 = 1100;

#[derive(Copy, Clone)]
pub enum TradeSignal {
    None,
    Sell1Buy2(u16),
    Buy1Sell2(u16),
}

pub struct SignalConfig {
    std_diapason_s: u16,
    hold_time_ms: u16,
    train_data_minutes: u16,
    train_interval_minutes: u16,
    min_profit_per_deal: f64,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            std_diapason_s: STD_DIAPASON_SECONDS,
            hold_time_ms: HOLD_TIME_MS,
            train_data_minutes: TRAIN_DATA_MINUTES,
            train_interval_minutes: TRAIN_INTERVAL_MINUTES,
            min_profit_per_deal: 1.0,
        }
    }
}

pub struct Signal {
    ticker1: String,
    ticker2: String,

    config: SignalConfig,

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
            config: SignalConfig::default(),

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
            self.values1_calculator.calculate(order_book, self.config.std_diapason_s)
        } else if self.ticker2 == ticker {
            self.values2_calculator.calculate(order_book, self.config.std_diapason_s)
        } else {
            return TradeSignal::None;
        };
        self.buffer.push(v);

        if let Some(calculator) = &self.calculator
            && let Some((v1, v2)) = self.get_last_values() {
            calculator.calculate(v1, v2, self.config.hold_time_ms)
        } else {
            TradeSignal::None
        }
    }
    
    pub fn calibrate(&mut self) {
        let train_values_size = self.get_train_values_size();
        if self.buffer.len() < train_values_size {
            return;
        }

        let last_time = self.buffer[self.buffer.len() - 1].time;
        if self.calculator.as_ref()
            .map(|calc| last_time - calc.get_created_on() > TimeDelta::minutes(self.config.train_interval_minutes as i64))
            .unwrap_or(true) {
            self.shrink_buffer();

            let hold_time = self.config.hold_time_ms;
            let min_profit_per_deal = self.config.min_profit_per_deal;

            let buf_clone = self.buffer.clone();
            let thread_up = thread::spawn(move || {
                match calibrate_signal_calculator(&buf_clone, true, hold_time, min_profit_per_deal) {
                    Ok(calc) => Some(calc),
                    Err(err) => {
                        error!("Failed to optimize UP calculator: {err}");
                        None
                    }
                }
            });

            let buf_clone = self.buffer.clone();
            let thread_down = thread::spawn(move || {
                match calibrate_signal_calculator(&buf_clone, false, hold_time, min_profit_per_deal) {
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
    
    fn get_train_values_size(&self) -> usize {
        self.config.train_data_minutes as usize * ORDER_BOOKS_PER_HOUR / 30
    }

    fn shrink_buffer(&mut self) {
        let train_values_size = self.get_train_values_size();
        if self.buffer.len() > train_values_size + 1 {
            let to_delete = self.buffer.len() - train_values_size - 1;
            self.buffer.drain(0..to_delete);
        }
    }
}