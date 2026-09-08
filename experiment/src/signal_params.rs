use crate::deal::{Deal, DealDirection};
use crate::simulation::{find_order_books_on_horizon, run_simulation};
use crate::{MarketEvent, OrderBookValues};
use argmin::core::{CostFunction, Error, Executor, State};
use argmin::solver::neldermead::NelderMead;
use chrono::TimeDelta;
use log::{info, warn};
use ndarray::Array1;

pub const IMBALANCE_LEVELS: [usize; 4] = [3, 5, 10, 20];
const DEFAULT_HOLD_MS: u16 = 250;
const SOLVER_ITERATIONS: u64 = 8192;

#[derive(Copy, Clone, Debug, Default)]
pub struct SignalParamsDir {
    pub threshold: f64,
    pub derivative1_weight: f64,
    pub derivative2_weight: f64,
    pub imbalance1_weights: [f64; IMBALANCE_LEVELS.len()],
    pub imbalance2_weights: [f64; IMBALANCE_LEVELS.len()],
}

impl SignalParamsDir {
    pub fn from_array(array: &Array1<f64>) -> Self {
        Self {
            threshold: 0.0,
            derivative1_weight: array[0].max(0.0),
            derivative2_weight: array[1],
            imbalance1_weights: [array[2], array[3], array[4], array[5]],
            imbalance2_weights: [array[6], array[7], array[8], array[9]],
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct SignalParams {
    pub hold_ms: u16, // 250
    pub up: Option<SignalParamsDir>,
    pub down: Option<SignalParamsDir>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Signal {
    None,
    Buy1Sell2,
    Sell1Buy2,
}

impl SignalParams {
    pub fn signal_score(&self, values1: &OrderBookValues, values2: &OrderBookValues) -> Option<f64> {
        let params = if values1.std_derivative > 0.0 {&self.up} else {&self.down};
        if let Some(params) = params {
            let mut signal = params.derivative1_weight * values1.std_derivative.abs()
                + params.derivative2_weight * values2.std_derivative;
            for i in 0..IMBALANCE_LEVELS.len() {
                signal += params.imbalance1_weights[i] * values1.imbalances[i]
                    + params.imbalance2_weights[i] * values2.imbalances[i];
            }
            Some(signal - params.threshold)
        } else { None }
    }

    pub fn signal(&self, values1: &OrderBookValues, values2: &OrderBookValues) -> Signal {
        if let Some(signal) = self.signal_score(values1, values2) {
            if signal > 0.0 {
                if values1.std_derivative < 0.0 {
                    Signal::Buy1Sell2
                } else {
                    Signal::Sell1Buy2
                }
            } else {
                Signal::None
            }
        } else {
            Signal::None
        }
    }
}

struct TradingProblem<'a> {
    direction: DealDirection,
    events: &'a [MarketEvent],
}

impl CostFunction for TradingProblem<'_> {
    type Param = Array1<f64>;
    type Output = f64;

    fn cost(&self, p: &Self::Param) -> Result<Self::Output, Error> {
        let dir_params = SignalParamsDir::from_array(p);
        let params = match self.direction {
            DealDirection::Sell1Buy2 => SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                up: Some(dir_params),
                down: None,
            },
            DealDirection::Buy1Sell2 => SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                up: None,
                down: Some(dir_params),
            },
        };
        let (signal_values, pnl_values) = collect_signal_to_pnl_values(self.events, &params);
        if signal_values.len() > 2 {
            Ok(-correlation::spearmanr(&pnl_values, &signal_values))
        } else {
            Err(Error::msg("Insufficient data"))
        }
    }
}

fn collect_signal_to_pnl_values(events: &[MarketEvent], params: &SignalParams) -> (Vec<f64>, Vec<f64>) {
    let mut signal_values = Vec::new();
    let mut pnl_values = Vec::new();
    let (mut last_order_book1, mut last_order_book2) = (None, None);

    for (i, event) in events.iter().enumerate() {
        match event {
            MarketEvent::OrderBook1(values) => last_order_book1 = Some(*values),
            MarketEvent::OrderBook2(values) => last_order_book2 = Some(*values),
            _ => {},
        }

        if let Some(v1) = &last_order_book1 && let Some(v2) = &last_order_book2 &&
            let Some(signal) = params.signal_score(v1, v2) {
            let mut deal = if v1.std_derivative > 0.0 {
                Deal::sell1_buy2(v1, v2)
            } else {
                Deal::buy1_sell2(v1, v2)
            };
            if let Some((fv1, fv2)) = find_order_books_on_horizon(
                events, i,
                deal.get_open_time() + TimeDelta::milliseconds(params.hold_ms as i64)
            ) {
                let price = match deal.get_direction() {
                    DealDirection::Sell1Buy2 => fv1.bid,
                    DealDirection::Buy1Sell2 => fv1.ask,
                };
                deal.close_instrument1(price, fv1.time);
                let price = match deal.get_direction() {
                    DealDirection::Sell1Buy2 => fv2.ask,
                    DealDirection::Buy1Sell2 => fv2.bid,
                };
                deal.close_instrument2(price, fv2.time);
                let (_, revenue, cost) = deal.close(false);
                signal_values.push(signal);
                pnl_values.push(revenue - cost);
            }
        }
    }
    (signal_values, pnl_values)
}

pub fn calibrate_params(events: &[MarketEvent]) -> SignalParams {
    let initial = Array1::from_vec(vec![
      //D1   D2   I3   I5   I10  I20  I3   I5   I10  I20
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0
    ]);

    let mut simplex = Vec::with_capacity(initial.len() + 1);

    simplex.push(initial.clone());

    for i in 0..initial.len() {
        let mut point = initial.clone();
        point[i] += match i {
            0 | 1 => 0.2,  // D1, D2
            _ => 0.1,      // Imbalance
        };
        simplex.push(point);
    }

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Sell1Buy2,
        events,
    };
    info!("Start optimization UP");
    let mut params_up = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(result.state().get_best_param().unwrap());
            info!("Best UP correlation {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };
    if let Some(threshold) = find_threshold(events, params_up, true) {
        params_up.threshold = threshold;
    } else {
        warn!("No threshold {:?}", params_up);
    }

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Buy1Sell2,
        events,
    };
    info!("Start optimization DOWN");
    let mut params_down = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(result.state().get_best_param().unwrap());
            info!("Best DOWN correlation {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };
    if let Some(threshold) = find_threshold(events, params_down, false) {
        params_down.threshold = threshold;
    } else {
        warn!("No threshold {:?}", params_down);
    }

    SignalParams {
        hold_ms: DEFAULT_HOLD_MS,
        up: Some(params_up),
        down: Some(params_down),
    }
}

fn find_threshold(events: &[MarketEvent], params_dir: SignalParamsDir, is_up: bool) -> Option<f64> {
    let params = SignalParams {
        hold_ms: DEFAULT_HOLD_MS,
        up: if is_up {Some(params_dir)} else {None},
        down: if !is_up {Some(params_dir)} else {None},
    };
    let (signal_values, pnl_values) = collect_signal_to_pnl_values(events, &params);
    let (mut min_signal, mut max_signal) = (None, None);
    for i in 0..signal_values.len() {
        if pnl_values[i] > 0.0 {
            let signal = signal_values[i];

            if let Some(cur_min) = min_signal {
                if signal < cur_min {
                    min_signal = Some(signal);
                }
            } else {
                min_signal = Some(signal);
            }

            if let Some(cur_max) = max_signal {
                if signal > cur_max {
                    max_signal = Some(signal);
                }
            } else {
                max_signal = Some(signal);
            }
        }
    }

    const LEVELS: usize = 8192;

    if let Some(min_signal) = min_signal && let Some(max_signal) = max_signal {
        info!("Signal min {min_signal} and max {max_signal}");
        let mut best_profit = f64::NAN;
        let mut best_threshold = 0.0;

        for i in 0..LEVELS {
            let threshold = min_signal + (max_signal - min_signal) * (i as f64) / LEVELS as f64;
            let mut test_params_dir = params_dir;
            test_params_dir.threshold = threshold;
            let test_params = SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                up: if is_up {Some(test_params_dir)} else {None},
                down: if !is_up {Some(test_params_dir)} else {None},
            };
            let result = run_simulation(events, &test_params);
            if result.win + result.loss > 100 {
                let net = result.income - result.outcome - result.commission;
                if best_profit.is_nan() || net > best_profit {
                    best_profit = net;
                    best_threshold = threshold;
                }
            }
        }
        if best_profit.is_nan() {
            None
        } else {
            info!("Best profit {best_profit} with threshold {best_threshold}");
            Some(best_threshold)
        }
    } else {
        None
    }
}