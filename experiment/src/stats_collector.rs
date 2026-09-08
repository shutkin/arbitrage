use crate::deal::{Deal, DealDirection};
use crate::signal_params::SignalParams;
use crate::simulation::{DealHandler, find_order_books_on_horizon, run_stats};
use crate::{COMMISSION_RATIO, MarketEvent};
use log::info;
use std::collections::HashMap;
use chrono::TimeDelta;

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
                    let commission = (revenue + cost) * COMMISSION_RATIO;
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