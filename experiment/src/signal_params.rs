use crate::{MarketEvent, OrderBookValues};
use crate::simulation::run_simulation;
use argmin::core::{CostFunction, Error, Executor, State};
use argmin::solver::neldermead::NelderMead;
use argmin::solver::particleswarm::ParticleSwarm;
use log::{error, info, warn};
use ndarray::Array1;
use crate::deal::DealDirection;

pub const IMBALANCE_LEVELS: [usize; 4] = [3, 5, 10, 20];
const DEFAULT_HOLD_MS: u16 = 300;// 250;
const SOLVER_ITERATIONS: u64 = 8192;

#[derive(Copy, Clone, Debug, Default)]
pub struct SignalParamsDir {
    pub alpha: f64,
    pub derivative1_weight: f64,
    pub derivative2_weight: f64,
    pub imbalance1_weights: [f64; IMBALANCE_LEVELS.len()],
    pub imbalance2_weights: [f64; IMBALANCE_LEVELS.len()],
}

impl SignalParamsDir {
    pub fn from_array(array: &Array1<f64>) -> Self {
        Self {
            alpha: array[0],
            derivative1_weight: array[1].max(0.0),
            derivative2_weight: array[2],
            imbalance1_weights: [array[3], array[4], array[5], array[6]],
            imbalance2_weights: [array[7], array[8], array[9], array[10]],
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct SignalParams {
    pub hold_ms: u16,
    pub signal_threshold: f64,
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
            let mut signal = params.alpha
                + params.derivative1_weight * values1.std_derivative.abs()
                + params.derivative2_weight * values2.std_derivative;
            for i in 0..IMBALANCE_LEVELS.len() {
                signal += params.imbalance1_weights[i] * values1.imbalances[i]
                    + params.imbalance2_weights[i] * values2.imbalances[i];
            }
            Some(signal)
        } else { None }
    }

    pub fn calc_signal(&self, values1: &OrderBookValues, values2: &OrderBookValues) -> Signal {
        if let Some(signal) = self.signal_score(values1, values2) {
            if signal > self.signal_threshold {
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
                signal_threshold: 0.0,
                up: Some(dir_params),
                down: None,
            },
            DealDirection::Buy1Sell2 => SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                signal_threshold: 0.0,
                up: None,
                down: Some(dir_params),
            },
        };
        let result = run_simulation(self.events, &params);
        let profit = result.income - result.outcome;// - result.commission;
        Ok(-profit)
    }
}

pub fn calibrate_params(events: &[MarketEvent]) -> SignalParams {
    let initial = Array1::from_vec(vec![
        -3.0,  // A
        1.0,  // B1
        0.0,  // B2
        0.0,  // C1
        0.0,  // C2
        0.0,  // C3
        0.0,  // C4
        0.0,  // C5
        0.0,  // C6
        0.0,  // C7
        0.0,  // C8
    ]);

    let mut simplex = Vec::with_capacity(initial.len() + 1);

    simplex.push(initial.clone());

    for i in 0..initial.len() {
        let mut point = initial.clone();

        point[i] += match i {
            0 => 1.0,      // A
            1 | 2 => 0.2,  // B1, B2
            _ => 0.1,      // C1..C8
        };

        simplex.push(point);
    }

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Sell1Buy2,
        events,
    };
    info!("Start optimization UP");
    let params_up = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(result.state().get_best_param().unwrap());
            info!("Best UP profit {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Buy1Sell2,
        events,
    };
    info!("Start optimization DOWN");
    let params_down = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(result.state().get_best_param().unwrap());
            info!("Best DOWN profit {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };

    SignalParams {
        hold_ms: DEFAULT_HOLD_MS,
        signal_threshold: 0.0,
        up: Some(params_up),
        down: Some(params_down),
    }
}

pub fn _calibrate_params(events: &[MarketEvent]) -> SignalParams {
    let bounds_down = Array1::from_vec(vec![-30.0, 0.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0]);
    let bounds_up = Array1::from_vec(vec![-5.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);

    let trade_problem = TradingProblem {
        direction: DealDirection::Sell1Buy2,
        events,
    };
    let solver = ParticleSwarm::<Array1<f64>, f64, _>::new((bounds_down.clone(), bounds_up.clone()), 64);
    info!("Start optimization UP");
    let params_up = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            if let Some(best_params) = result.state().get_best_param() {
                //let best_cost = result.state().get_best_cost();
                let best_cost = best_params.cost;
                let params = SignalParamsDir::from_array(&best_params.position);
                info!("Best UP profit {} with {params:?}", -best_cost);
                Some(params)
            } else {
                warn!("No best up params found");
                None
            }
        },
        Err(e) => {
            error!("{e}");
            None
        }
    };

    let trade_problem = TradingProblem {
        direction: DealDirection::Buy1Sell2,
        events,
    };
    let solver = ParticleSwarm::<Array1<f64>, f64, _>::new((bounds_down, bounds_up), 64);
    info!("Start optimization DOWN");
    let params_down = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            if let Some(best_params) = result.state().get_best_param() {
                //let best_cost = result.state().get_best_cost();
                let best_cost = best_params.cost;
                let params = SignalParamsDir::from_array(&best_params.position);
                info!("Best DOWN profit {} with {params:?}", -best_cost);
                Some(params)
            } else {
                warn!("No best up params found");
                None
            }
        },
        Err(e) => {
            error!("{e}");
            None
        }
    };

    SignalParams {
        hold_ms: DEFAULT_HOLD_MS,
        signal_threshold: 0.0,
        up: params_up,
        down: params_down,
    }
}
