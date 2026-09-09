use crate::OrderBookValues;
use chrono::TimeDelta;

pub const DEVIATION_MAX_VALUE: f64 = 100.0;

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
            return if derivative < 0.0 { -DEVIATION_MAX_VALUE } else { DEVIATION_MAX_VALUE };
        }
        let n = derivative / std;
        n
        //n.clamp(-DEVIATION_MAX_VALUE, DEVIATION_MAX_VALUE)
        //n / DEVIATION_MAX_VALUE
        //let l = (n.abs() * 2.0 / DEVIATION_MAX_VALUE).log2();
        //if n < 0.0 {-l} else {l}
    } else {
        0.0
    }
}

pub fn percentile(values: &[OrderBookValues], i: usize) -> f64 {
    if i < 3 {
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
    window_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let derivative = calc_derivative(values, i);
    let mut window_index = 0;
    while window_index < window_values.len() && derivative < window_values[window_index] {
        window_index += 1;
    }
    let x = window_index as f64 / window_values.len() as f64;
    (x * 2.0).log2()
}

fn mad_normalize(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    if v.len() < 3 {
        return v;
    }

    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = v[v.len() / 2];

    let mut deviations = v.into_iter().map(|v| (v - median).abs()).collect::<Vec<_>>();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad = deviations[deviations.len() / 2];

    let mut scale = 1.4826 * mad;
    if scale < f64::MIN_POSITIVE {
        scale = 1.0;
    }
    values.iter().map(|&v| (v - median) / scale).collect()
}

pub fn huber_loss(x: &[f64], target: &[f64], delta: f64) -> f64 {
    let target = mad_normalize(target);
    let mut result = 0.0;
    x.iter().zip(target.iter()).for_each(|(x, target)| {
        let r = target - x;
        let r_abs = r.abs();
        result += if r_abs <= delta {
            0.5 * r * r
        } else {
            delta * (r_abs - 0.5 * delta)
        };
    });
    result / x.len() as f64
}

#[cfg(test)]
mod tst {
#[test]
    fn tstexp() {
        for i in 0..7 {
            let x = 0.5 + 0.5 * i as f64 / 7.0;
            let y = (x * 2.0).log2();
            println!("{x} -> {y}");
        }
    }
}
