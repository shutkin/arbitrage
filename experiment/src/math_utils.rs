use chrono::TimeDelta;
use log::warn;
use crate::OrderBookValues;

fn standard_deviation(data: &[f64]) -> Option<f64> {
    let count = data.len();
    if count == 0 {
        return None;
    }

    let mean = data.iter().sum::<f64>() / count as f64;
    let variance = data.iter()
        .map(|value| {
            let diff = value - mean;
            diff * diff
        })
        .sum::<f64>() / count as f64;

    Some(variance.sqrt())
}

fn calc_derivative(values: &[OrderBookValues], i: usize) -> f64 {
    if i < 1 {
        return 0.0;
    }

    let mut pi = i - 1;
    while values[pi].time == values[i].time {
        if pi == 0 {
            return 0.0;
        }
        pi -= 1;
    }
    (values[i].mid - values[pi].mid) / (values[i].time.timestamp_millis() - values[pi].time.timestamp_millis()) as f64
}

pub fn std_derivative(values: &[OrderBookValues], i: usize) -> f64 {
    if i < 2 {
        return 0.0;
    }

    let mut pi = i - 1;
    let avg_period_start = values[i].time - TimeDelta::seconds(5 * 60);
    let mut window_values = Vec::new();
    while pi > 1 && values[pi].time > avg_period_start {
        let derivative = calc_derivative(values, pi);
        window_values.push(derivative);
        pi -= 1;
    }
    if let Some(std) = standard_deviation(&window_values) {
        let derivative = calc_derivative(values, i);
        if std.abs() < 0.0000001 {
            return if derivative < 0.0 { -70.0 } else { 70.0 };
        }
        let n = derivative / std;
        if n.abs() > 70.0 {
            warn!("Abnormal std {n}: {} / {}", derivative, std);
        }
        n.clamp(-70.0, 70.0)
    } else {
        0.0
    }
}
