use crate::orderbook_values::OrderBookValues;
use crate::signal_calculator::{CalculatorsPair, SignalCalculator};
use crate::simulation::run_simulation;
use argmin::core::{CostFunction, Error, Executor, State};
use argmin::solver::neldermead::NelderMead;
use log::debug;
use model::common::CommonError;
use ndarray::Array1;

struct TradeSimProblem<'a> {
    values: &'a [OrderBookValues],
    is_up: bool,
    hold_time_ms: u16,
}

impl CostFunction for TradeSimProblem<'_> {
    type Param = Array1<f64>;
    type Output = f64;

    fn cost(&self, param: &Self::Param) -> Result<Self::Output, Error> {
        let last_time = self.values[self.values.len() - 1].time;
        let calculator = SignalCalculator::from_optimizer_param(param);
        let pair = if self.is_up {
            CalculatorsPair::new_up(calculator, last_time)
        } else {
            CalculatorsPair::new_down(calculator, last_time)
        };

        let profit_per_deal = run_simulation(self.values, &pair, self.hold_time_ms);
        Ok(-profit_per_deal)
    }
}

pub fn calibrate_signal_calculator(
    values: &[OrderBookValues],
    is_up: bool,
    hold_time_ms: u16,
    min_profit_per_deal: f64,
) -> Result<Option<SignalCalculator>, CommonError> {
    let initial = Array1::from_vec(
        //      A    D1   D2   I3   I5   I10  I20  I50  I3   I5   I10  I20  I50
        vec![3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );

    let mut simplex = Vec::with_capacity(initial.len() + 1);

    simplex.push(initial.clone());

    for i in 0..initial.len() {
        let mut point = initial.clone();
        point[i] += match i {
            0 => 1.0,      // A
            1 | 2 => 0.2,  // D1, D2
            _ => 0.1,      // Imbalance
        };
        simplex.push(point);
    }

    let solver = NelderMead::<Array1<f64>, f64>::new(simplex.clone());
    let problem = TradeSimProblem { values, is_up, hold_time_ms };

    debug!("Start optimization {} on events {} - {}",
        if is_up {"UP"} else {"DOWN"}, values[0].time, values[values.len() - 1].time);
    let result = Executor::new(problem, solver)
        .configure(|state| state.max_iters(4096))
        .run()?;
    let best_profit = -result.state().get_best_cost();
    debug!("{} best profit {best_profit}", if is_up {"UP"} else {"DOWN"});
    if best_profit > min_profit_per_deal {
        let param = result.state().get_best_param().ok_or("Failed to get best param")?;
        Ok(Some(SignalCalculator::from_optimizer_param(param)))
    } else {
        Ok(None)
    }
}