use crate::TradeSignal;
use crate::orderbook_values::{IMBALANCE_LEVELS, OrderBookValues};
use chrono::{DateTime, Utc};
use ndarray::Array1;

#[derive(Copy, Clone, Debug)]
pub struct SignalCalculator {
    threshold: f64,
    derivative1_weight: f64,
    derivative2_weight: f64,
    imbalance1_weights: [f64; IMBALANCE_LEVELS.len()],
    imbalance2_weights: [f64; IMBALANCE_LEVELS.len()],
}

impl SignalCalculator {
    pub fn from_optimizer_param(param: &Array1<f64>) -> Self {
        Self {
            threshold: param[0],
            derivative1_weight: param[1].max(0.0),
            derivative2_weight: param[2],
            imbalance1_weights: [param[3], param[4], param[5], param[6], param[7]],
            imbalance2_weights: [param[8], param[9], param[10], param[11], param[12]],
        }
    }

    fn calculate(&self, v1: &OrderBookValues, v2: &OrderBookValues) -> f64 {
        let mut score = self.derivative1_weight * v1.normal_derivative.abs()
            + self.derivative2_weight * v2.normal_derivative;
        for i in 0..IMBALANCE_LEVELS.len() {
            score += self.imbalance1_weights[i] * v1.imbalances[i]
                + self.imbalance2_weights[i] * v2.imbalances[i];
        }
        score - self.threshold
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CalculatorsPair {
    created_on: DateTime<Utc>,
    up: Option<SignalCalculator>,
    down: Option<SignalCalculator>,
}

impl CalculatorsPair {
    pub fn new_up(up: SignalCalculator, last_time: DateTime<Utc>) -> Self {
        Self {
            created_on: last_time,
            up: Some(up),
            down: None,
        }
    }
    
    pub fn new_down(down: SignalCalculator, last_time: DateTime<Utc>) -> Self {
        Self {
            created_on: last_time,
            up: None,
            down: Some(down),
        }
    }
    
    pub fn new(up: Option<SignalCalculator>, down: Option<SignalCalculator>, last_time: DateTime<Utc>) -> Self {
        Self {
            created_on: last_time,
            up,
            down,
        }
    }
    
    pub fn get_created_on(&self) -> DateTime<Utc> {
        self.created_on
    }
    
    pub fn calculate(&self, v1: &OrderBookValues, v2: &OrderBookValues, hold_time_ms: u16) -> TradeSignal {
        let calculator = if v1.normal_derivative > 0.0 {&self.up} else {&self.down};
        let score = calculator.map(|calc| calc.calculate(v1, v2)).unwrap_or(0.0);
        if score > 0.0 {
            if v1.normal_derivative > 0.0 {
                TradeSignal::Sell1Buy2(hold_time_ms)
            } else {
                TradeSignal::Buy1Sell2(hold_time_ms)
            }
        } else {
            TradeSignal::None
        }
    }
}

pub fn def_calculator(time: DateTime<Utc>) -> CalculatorsPair {
    CalculatorsPair {
        created_on: time,
        up: Some(SignalCalculator { threshold: 6.653802170662007, derivative1_weight: 0.1838496911725353, derivative2_weight: -0.1630741886777645, imbalance1_weights: [-0.1816349844788731, 0.11941825272636117, 0.04399864656050896, 0.22209138454769906, 0.02391486403417551], imbalance2_weights: [-0.10229728876048462, 0.17752644262292283, -0.08827349607286118, -0.05575711617335116, 0.11033908081509058] }),
        down: Some(SignalCalculator { threshold: 6.418031476549265, derivative1_weight: 0.2392164161790894, derivative2_weight: 0.05530916253476989, imbalance1_weights: [-0.127381108188862, 0.10278265467643305, 0.17807838540013002, 0.19681729665979042, 0.07584425323259014], imbalance2_weights: [0.02027717587950075, -0.03154636125652479, -0.10007531634593625, -0.06466022429683423, -0.13893706926813398] }) }
}