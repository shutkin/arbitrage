use crate::OrderBookValues;
use crate::simulation::{DealDirection, run_simulation};
use argmin::core::{CostFunction, Error, Executor};
use argmin::solver::neldermead::NelderMead;
use log::info;
use ndarray::Array1;

pub const IMBALANCE_LEVELS: [usize; 4] = [3, 5, 10, 20];
const DEFAULT_HOLD_MS: u16 = 300;
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
            threshold: array[0] * 10.0,
            derivative1_weight: array[1] * 10.0,
            derivative2_weight: array[2] * 10.0,
            imbalance1_weights: [array[3], array[4], array[5], array[6]],
            imbalance2_weights: [array[7], array[8], array[9], array[10]],
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct SignalParams {
    pub hold_ms: u16,
    pub up: Option<SignalParamsDir>,
    pub down: Option<SignalParamsDir>,
}

impl SignalParams {
    pub fn calc_signal(&self, values1: &OrderBookValues, values2: &OrderBookValues) -> f64 {
        let params = if values1.std_derivative > 0.0 {&self.up} else {&self.down};
        if let Some(params) = params {
            let mut result = params.threshold
                + params.derivative1_weight * values1.std_derivative.abs()
                + params.derivative2_weight * values2.std_derivative;
            for i in 0..IMBALANCE_LEVELS.len() {
                result += params.imbalance1_weights[i] * values1.imbalances[i]
                    + params.imbalance2_weights[i] * values2.imbalances[i];
            }
            result
        } else {
            0.0
        }
    }
}

struct TradingProblem<'a> {
    direction: DealDirection,
    values1: &'a [OrderBookValues],
    values2: &'a [OrderBookValues],
}

impl CostFunction for TradingProblem<'_> {
    type Param = Array1<f64>;
    type Output = f64;

    fn cost(&self, p: &Self::Param) -> Result<Self::Output, Error> {
        let params = match self.direction {
            DealDirection::Sell1Buy2 => SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                up: Some(SignalParamsDir::from_array(p)),
                down: None,
            },
            DealDirection::Buy1Sell2 => SignalParams {
                hold_ms: DEFAULT_HOLD_MS,
                up: None,
                down: Some(SignalParamsDir::from_array(p)),
            },
        };
        let result = run_simulation(self.values1, self.values2, &params);
        let profit = result.income - result.outcome - result.commission;
        Ok(-profit)
    }
}

pub fn calibrate_params(values1: &[OrderBookValues], values2: &[OrderBookValues]) -> SignalParams {
    let initial = Array1::from_vec(vec![
        -1.0,  // A
        0.5,  // B1
        -0.1,  // B2
        0.1,  // C1
        0.2,  // C2
        0.1,  // C3
        -0.1,  // C4
        0.1,  // C5
        0.2,  // C6
        0.1,  // C7
        -0.1,  // C8
    ]);

    let mut simplex = Vec::with_capacity(initial.len() + 1);

    simplex.push(initial.clone());

    for i in 0..initial.len() {
        let mut point = initial.clone();
        point[i] += 0.015;
        simplex.push(point);
    }

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Sell1Buy2,
        values1,
        values2,
    };
    info!("Start optimization UP");
    let params_up = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(&result.state().best_param.clone().unwrap_or_default());
            info!("Best UP profit {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let trade_problem = TradingProblem {
        direction: DealDirection::Buy1Sell2,
        values1,
        values2,
    };
    info!("Start optimization DOWN");
    let params_down = match Executor::new(trade_problem, solver)
        .configure(|state| state.max_iters(SOLVER_ITERATIONS))
        .run() {
        Ok(result) => {
            let best_cost = result.state().get_best_cost();
            let params = SignalParamsDir::from_array(&result.state().best_param.clone().unwrap_or_default());
            info!("Best DOWN profit {} with {params:?}", -best_cost);
            params
        },
        Err(e) => panic!("{e}"),
    };

    SignalParams {
        hold_ms: DEFAULT_HOLD_MS,
        up: Some(params_up),
        down: Some(params_down),
    }
}
