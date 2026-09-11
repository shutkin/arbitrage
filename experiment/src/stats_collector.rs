use crate::deal::{Deal, DealDirection};
use crate::math_utils::{mean, median, positive_percentile, standard_deviation};
use crate::signal_optimization::{calibrate_params, collect_signal_to_pnl_values, SignalParams, SignalPnL, DEFAULT_HOLD_MS, CostFunctionImpl};
use crate::signals::{signal_huber_09_07, signal_spearman_09_07, signal_trading_09_07, signal_unknown_09_03};
use crate::simulation::{DealHandler, find_order_books_on_horizon, run_stats};
use crate::{commission, MarketEvent};
use chrono::{TimeDelta, Timelike};
use log::info;
use std::collections::HashMap;

pub trait StatisticsProvider {
    fn variants(&self) -> Vec<String>;
    fn run(&mut self, variant: u8) -> Vec<(String, String)>;
}

fn collect_to_table(provider: &mut dyn StatisticsProvider) -> String {
    let variants = provider.variants();
    let mut table_rows = Vec::new();
    table_rows.push(format!("v|{}", variants.join("|")));
    table_rows.push((0..=variants.len()).map(|_| "-").collect::<Vec<_>>().join("|"));
    for i in 0..variants.len() as u8 {
        info!("Run variant {}", variants[i as usize]);
        let rows = provider.run(i);
        if i == 0 {
            for (name, value) in rows {
                table_rows.push(format!("{name}|{value}"));
            }
        } else {
            for (i, (_, value)) in rows.into_iter().enumerate() {
                table_rows[i + 2].push_str(&format!("|{value}"));
            }
        }
    }
    table_rows.join("\n")
}

pub fn threshold_horizon_probabilities(events: &[MarketEvent], params: SignalParams) -> String {
    const HORIZONS: [u32; 11] = [250, 500, 1000, 2000, 5000, 10000, 30000, 60000, 300000, 600000, 1800000];

    struct Handler {
        map: HashMap<u32, Vec<bool>>,
    }

    impl DealHandler for Handler {
        fn handle_deal(&mut self, events: &[MarketEvent], event_index: usize, deal: Deal) {
            for horizon in HORIZONS {
                if let Some((v1, v2)) = find_order_books_on_horizon(
                    events, event_index, deal.get_open_time() + TimeDelta::milliseconds(horizon as i64)
                ) {
                    let mut deal_copy = deal;
                    match deal_copy.get_direction() {
                        DealDirection::Sell1Buy2 => {
                            deal_copy.close_instrument1(v1.ask, v1.time);
                            deal_copy.close_instrument2(v2.bid, v2.time);
                        },
                        DealDirection::Buy1Sell2 => {
                            deal_copy.close_instrument1(v1.bid, v1.time);
                            deal_copy.close_instrument2(v2.ask, v2.time);
                        }
                    }
                    let (_, revenue, cost) = deal_copy.close(false);
                    let commission = commission(revenue, cost);
                    self.map.get_mut(&horizon).unwrap().push(revenue - cost > commission);
                }
            }
        }

        fn get_stats(&self) -> Vec<(String, String)> {
            HORIZONS.iter().map(|horizon| {
                let v = self.map.get(horizon).unwrap();
                let positive = v.iter().filter(|x| **x).count();
                let probability = 100.0 * positive as f64 / v.len() as f64;
                (horizon.to_string(), format!("{probability:.3}% of {}", v.len()))
            }).collect()
        }

        fn init(&mut self, _variant: u8) {
            for horizon in HORIZONS {
                self.map.insert(horizon, Vec::<bool>::new());
            }
        }
    }

    let mut handler = Handler { map: HashMap::new() };

    struct Runner<'a> {
        pub events: &'a [MarketEvent],
        pub params: SignalParams,
        pub deal_handler: &'a mut dyn DealHandler,
    }

    impl StatisticsProvider for Runner<'_> {
        fn variants(&self) -> Vec<String> {
            vec!["threshold 0".to_string(), "threshold 5".to_string(), "threshold 10".to_string()]
        }

        fn run(&mut self, variant: u8) -> Vec<(String, String)> {
            /*self.params.signal_threshold = match variant {
                1 => 5.0,
                2 => 10.0,
                _ => 0.0,
            };*/
            self.deal_handler.init(variant);
            run_stats(self.events, &self.params, self.deal_handler)
        }
    }

    let mut runner = Runner {
        events,
        params,
        deal_handler: &mut handler,
    };

    collect_to_table(&mut runner)
}

pub fn signal_after_deal(events: &[MarketEvent], params: SignalParams) -> String {
    const HORIZONS: [u32; 9] = [10, 50, 100, 200, 500, 1000, 2000, 5000, 10000];

    enum StatVariant {
        Mean, Median,
    }

    struct Handler {
        map: HashMap<u32, Vec<f64>>,
        params: SignalParams,
        variant: StatVariant,
    }

    impl DealHandler for Handler {
        fn handle_deal(&mut self, events: &[MarketEvent], event_index: usize, deal: Deal) {
            for horizon in HORIZONS {
                if let Some((v1, v2)) = find_order_books_on_horizon(
                    events, event_index, deal.get_open_time() + TimeDelta::milliseconds(horizon as i64)
                ) {
                    let score = self.params.signal_score(&v1, &v2).unwrap_or(0.0);
                    self.map.get_mut(&horizon).unwrap().push(score);
                }
            }
        }

        fn get_stats(&self) -> Vec<(String, String)> {
            HORIZONS.iter().map(|horizon| {
                let mut values = self.map.get(horizon).cloned().unwrap_or_default();
                let v = match self.variant {
                    StatVariant::Mean => {
                        values.iter().sum::<f64>() / values.len() as f64
                    }
                    StatVariant::Median => {
                        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        values[values.len() / 2]
                    }
                };
                (horizon.to_string(), format!("{v:.2}"))
            }).collect()
        }

        fn init(&mut self, variant: u8) {
            self.variant = match variant {
                1 | 3 | 5 => StatVariant::Median,
                _ => StatVariant::Mean
            };
            for horizon in HORIZONS {
                self.map.insert(horizon, Vec::new());
            }
        }
    }

    let mut handler = Handler {
        map: HashMap::new(),
        params,
        variant: StatVariant::Mean,
    };

    struct Runner<'a> {
        pub events: &'a [MarketEvent],
        pub params: SignalParams,
        pub deal_handler: &'a mut dyn DealHandler,
    }

    impl StatisticsProvider for Runner<'_> {
        fn variants(&self) -> Vec<String> {
            vec!["threshold 0 mean".to_string(), "threshold 0 median".to_string(),
                 "threshold 5 mean".to_string(), "threshold 5 median".to_string(),
                 "threshold 10 mean".to_string(), "threshold 10 median".to_string(),]
        }

        fn run(&mut self, variant: u8) -> Vec<(String, String)> {
            /*self.params.signal_threshold = match variant {
                2 | 3 => 5.0,
                4 | 5 => 10.0,
                _ => 0.0,
            };*/
            self.deal_handler.init(variant);
            run_stats(self.events, &self.params, self.deal_handler)
        }
    }

    let mut runner = Runner {
        events,
        params,
        deal_handler: &mut handler,
    };

    collect_to_table(&mut runner)
}

pub fn daily_signal_to_pnl(daily_events: Vec<Vec<MarketEvent>>) -> String {
    const PERCENTILES: [f64; 5] = [0.05, 0.02, 0.01, 0.005, 0.002];

    struct Runner {
        day_titles: Vec<String>,
        values: Vec<SignalPnL>,
        signal_sorted: Vec<Vec<f64>>,
    }

    impl Runner {
        pub fn new(daily_events: Vec<Vec<MarketEvent>>) -> Self {
            let mut day_titles = Vec::new();
            let mut daily_values = Vec::new();
            let mut daily_signal_sorted = Vec::new();

            for i in 1..daily_events.len() {
                let mut train_events = Vec::new();
                for back in 0..3 {
                    let train_day = i as i32 + back - 3;
                    if train_day >= 0 {
                        train_events.extend_from_slice(&daily_events[train_day as usize]);
                    }
                }
                day_titles.push(format!("{}", daily_events[i][0].event_datetime().date_naive()));

                let params = calibrate_params(&train_events, CostFunctionImpl::HuberLoss);
                let values = collect_signal_to_pnl_values(&daily_events[i], &params);
                let mut signal_sorted = values.signal.clone();
                signal_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                daily_values.push(values);
                daily_signal_sorted.push(signal_sorted);
            }

            Self {
                day_titles,
                values: daily_values,
                signal_sorted: daily_signal_sorted,
            }
        }
    }

    impl StatisticsProvider for Runner {
        fn variants(&self) -> Vec<String> {
            PERCENTILES.iter().flat_map(|p| vec![format!("{p}% total PnL"), format!("{p}% median PnL"), format!("{p}% win rate")]).collect()
        }

        fn run(&mut self, variant: u8) -> Vec<(String, String)> {
            let mut result = Vec::new();
            for (day, (values, signal_sorted)) in self.values.iter().zip(self.signal_sorted.iter()).enumerate() {
                let percentile = PERCENTILES[variant as usize / 3];
                let offset_from_top = (signal_sorted.len() as f64 * percentile / 100.0).round() as usize;
                let threshold = signal_sorted[signal_sorted.len() - 1 - offset_from_top];
                let mut pnl_values = Vec::new();
                for (i, signal) in values.signal.iter().enumerate() {
                    if *signal >= threshold {
                        pnl_values.push(values.pnl[i] - values.commission[i]);
                    }
                }
                let date = self.day_titles[day].clone();
                let v = match variant % 3 {
                    0 => format!("{:.3}", pnl_values.iter().sum::<f64>()),
                    1 => format!("{:.3}", median(&pnl_values)),
                    _ => format!("{:.2}%", positive_percentile(&pnl_values)),
                };
                result.push((date, v));
            }
            result
        }
    }

    let mut runner = Runner::new(daily_events);
    collect_to_table(&mut runner)
}

pub fn signal_to_future_pnl_advances(train_events: &[MarketEvent], test_events: &[MarketEvent]) -> Vec<String> {
    const PERCENTILES: [f64; 15] = [50.0, 20.0, 10.0, 5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01, 0.005, 0.002, 0.001];

    struct Runner {
        values: [SignalPnL; 2],
        signal_sorted: [Vec<f64>; 2],
    }

    impl Runner {
        pub fn new(train_events: &[MarketEvent], test_events: &[MarketEvent], params: &SignalParams) -> Self {
            let train_values = collect_signal_to_pnl_values(train_events, &params);
            let mut train_signal_sorted = train_values.signal.clone();
            train_signal_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

            let test_values = collect_signal_to_pnl_values(test_events, &params);
            let mut test_signal_sorted = test_values.signal.clone();
            test_signal_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

            Self {
                values: [train_values, test_values],
                signal_sorted: [train_signal_sorted, test_signal_sorted],
            }
        }
    }

    impl StatisticsProvider for Runner {
        fn variants(&self) -> Vec<String> {
            vec!["N".to_string(), "mean PnL".to_string(), "median PnL".to_string(),
                 "std PnL".to_string(), "win rate".to_string(), "Signal threshold".to_string(),]
        }

        fn run(&mut self, variant: u8) -> Vec<(String, String)> {
            PERCENTILES.iter().map(|percentile| {
                let mut threshold = [0.0, 0.0];
                let mut pnl_values = [Vec::new(), Vec::new()];
                for data_set in 0..2 {
                    let offset_from_top = (self.signal_sorted[data_set].len() as f64 * percentile / 100.0).round() as usize;
                    threshold[data_set] = self.signal_sorted[data_set][self.signal_sorted[data_set].len() - 1 - offset_from_top];

                    for (i, signal) in self.values[data_set].signal.iter().enumerate() {
                        if *signal >= threshold[data_set] {
                            pnl_values[data_set].push(self.values[data_set].pnl[i] - self.values[data_set].commission[i]);
                        }
                    }
                }

                let percentile_str = format!("{}%", percentile);
                let v = match variant {
                    0 => format!("{} / {}", pnl_values[0].len(), pnl_values[1].len()),
                    1 => format!("{:.3} / {:.3}", mean(&pnl_values[0]), mean(&pnl_values[1])),
                    2 => format!("{:.3} / {:.3}", median(&pnl_values[0]), median(&pnl_values[1])),
                    3 => format!("{:.3} / {:.3}", standard_deviation(&pnl_values[0]).unwrap_or_default(), standard_deviation(&pnl_values[1]).unwrap_or_default()),
                    4 => format!("{:.3} / {:.3}", positive_percentile(&pnl_values[0]), positive_percentile(&pnl_values[1])),
                    _ => format!("{:.4} / {:.4}", threshold[0], threshold[1]),
                };
                (percentile_str, v)
            }).collect()
        }
    }
    
    let mut result = Vec::new();

    let params = SignalParams {hold_ms: DEFAULT_HOLD_MS, up: None, down: signal_trading_09_07().down};
    let mut runner = Runner::new(train_events, test_events, &params);
    result.push(collect_to_table(&mut runner));
    
    //let params = SignalParams {hold_ms: DEFAULT_HOLD_MS, up: signal_spearman_09_07().up, down: None};
    //let mut runner = Runner::new(train_events, test_events, &params);
    //result.push(collect_to_table(&mut runner));
    
    let params = SignalParams {hold_ms: DEFAULT_HOLD_MS, up: None, down: signal_huber_09_07().down};
    let mut runner = Runner::new(train_events, test_events, &params);
    result.push(collect_to_table(&mut runner));
    
    result
}

pub fn signal_to_future_pnl_relation(events: &[MarketEvent]) -> String {
    struct Runner<'a> {
        pub events: &'a [MarketEvent],
    }

    impl StatisticsProvider for Runner<'_> {
        fn variants(&self) -> Vec<String> {
            vec!["simulation UP".to_string(), "simulation DOWN".to_string(),
                 "Spearman UP".to_string(), "Spearman DOWN".to_string(),
                 "Huber UP".to_string(), "Huber DOWN".to_string()]
        }

        fn run(&mut self, variant: u8) -> Vec<(String, String)> {
            let params = match variant {
                0 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: signal_trading_09_07().up, down: None },
                1 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: None, down: signal_trading_09_07().down },
                2 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: signal_spearman_09_07().up, down: None },
                3 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: None, down: signal_spearman_09_07().down },
                4 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: signal_huber_09_07().up, down: None },
                5 => SignalParams { hold_ms: DEFAULT_HOLD_MS, up: None, down: signal_huber_09_07().down },
                _ => signal_unknown_09_03(),
            };
            let values = collect_signal_to_pnl_values(self.events, &params);
            if values.signal.len() < 100 {
                return (0..20).map(|i| (i.to_string(), "-".to_string())).collect();
            }

            let mut s = values.signal.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let min = s[s.len() / 10];
            let max = s[s.len() * 9 / 10];

            (0..20).map(|i| (i, min + (max - min) * i as f64 / 20.0 .. min + (max - min) * (i + 1) as f64 / 20.0))
                .map(|(index, range)| {
                    let mut signal_pnl = Vec::new();
                    let (mut win, mut loss) = (0, 0);
                    for (i, signal) in values.signal.iter().enumerate() {
                        if range.contains(signal) {
                            signal_pnl.push(values.pnl[i]);
                            if values.pnl[i] > 0.0 {
                                win += 1;
                            } else {
                                loss += 1;
                            }
                        }
                    }
                    signal_pnl.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let mean = signal_pnl.iter().sum::<f64>() / signal_pnl.len() as f64;
                    let p25 = signal_pnl[signal_pnl.len() / 4];
                    let p75 = signal_pnl[signal_pnl.len() * 3 / 4];
                    let win_loss = 100.0 * win as f64 / (win + loss) as f64;
                    (index.to_string(), format!("{:.3} ({:.3} - {:.3}) {:.2}% {}", mean, p25, p75, win_loss, win + loss))
                }).collect()
        }
    }

    let mut runner = Runner { events };
    collect_to_table(&mut runner)
}