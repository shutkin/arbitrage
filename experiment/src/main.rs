mod order_book_cache;
mod signal_optimization;
mod math_utils;
mod simulation;
mod stats_collector;
mod deal;
mod signals;

use crate::deal::DealDirection;
use crate::math_utils::{mean, median, standard_deviation};
use crate::signal_optimization::{CostFunctionImpl, IMBALANCE_LEVELS, calibrate_params};
use crate::signals::signal_huber_09_03;
use crate::simulation::run_simulation;
use crate::stats_collector::{daily_signal_to_pnl, deviation_to_spread_movement, trend_buckets};
use chrono::{DateTime, Datelike, TimeDelta, Utc};
use db::{Db, QueryAsksOrBids};
use log::info;
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::{Instrument, OrderBook, Trade};
use simplelog::{LevelFilter, SimpleLogger};

pub fn commission(revenue: f64, cost: f64) -> f64 {
    // T-Invest
    //(revenue + cost) * 0.015 / 100.0

    // Alor
    5.0
}

fn find_instrument_id(instruments: &[Instrument], ticker: &str) -> Option<i16> {
    instruments
        .iter()
        .find(|instrument| instrument.ticker == ticker)
        .and_then(|instrument| instrument.id)
}

async fn get_order_books(tickers: &[&str], ids: &[i16], diapason: TimeDiapason, db: Option<&Db>)
                         -> Result<(Vec<OrderBook>, Vec<OrderBook>), CommonError> {
    if let (Some(order_books1), Some(order_books2)) =
        (order_book_cache::read(tickers[0], diapason, true)?,
         order_book_cache::read(tickers[1], diapason, true)?) {
        Ok((order_books1, order_books2))
    } else {
        let (order_books1, order_books2) =
            (db.unwrap().get_order_books(Some(ids[0]), diapason, QueryAsksOrBids::BOTH).await?,
             db.unwrap().get_order_books(Some(ids[1]), diapason, QueryAsksOrBids::BOTH).await?);
        order_book_cache::write(tickers[0], diapason, &order_books1, true)?;
        order_book_cache::write(tickers[1], diapason, &order_books2, true)?;
        Ok((order_books1, order_books2))
    }
}

#[tokio::main]
async fn __main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let mut diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-03T05:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")?.to_utc(),
    );

    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        let mut daily_events = Vec::new();

        for day in 3..12 {
            if !matches!(diapason.from.weekday().number_from_monday(), 6 | 7) {
                info!("DAY {day}");
                let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], diapason, Some(&db)).await?;
                let (mut all_values1, mut all_values2) = (Vec::new(), Vec::new());
                convert_values(&order_books1, &order_books2, &mut all_values1, &mut all_values2);
                info!("Calculate std deviations");
                calculate_std_deviations(&mut all_values1);
                calculate_std_deviations(&mut all_values2);
                let day_events = merge_events(&all_values1, &all_values2, &[], &[]);
                daily_events.push(day_events);
            }

            diapason.from += TimeDelta::days(1);
            diapason.to += TimeDelta::days(1);
        }

        let events_clone = daily_events.clone();
        std::thread::spawn(move || {
            info!("UP:\n{}\n", daily_signal_to_pnl(events_clone, DealDirection::Sell1Buy2));
        });
        info!("DOWN:\n{}\n", daily_signal_to_pnl(daily_events, DealDirection::Buy1Sell2));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let mut diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-03T05:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")?.to_utc(),
    );

    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        let mut daily_events = Vec::new();

        for day in 3..12 {
            if !matches!(diapason.from.weekday().number_from_monday(), 6 | 7) {
                info!("DAY {day}");
                let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], diapason, Some(&db)).await?;
                let (mut all_values1, mut all_values2) = (Vec::new(), Vec::new());
                convert_values(&order_books1, &order_books2, &mut all_values1, &mut all_values2);

                info!("Calculate std deviations");
                calculate_std_deviations(&mut all_values1);
                calculate_std_deviations(&mut all_values2);

                info!("Calculate trend");
                calculate_trend(&mut all_values1);
                calculate_trend(&mut all_values2);

                let day_events = merge_events(&all_values1, &all_values2, &[], &[]);
                daily_events.push((diapason.from.date_naive().to_string(), day_events));
            }

            diapason.from += TimeDelta::days(1);
            diapason.to += TimeDelta::days(1);
        }
        info!("\n{}", trend_buckets(&daily_events));
    }
    Ok(())
}

#[tokio::main]
async fn _main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let train_diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-03T05:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")?.to_utc(),
    );
    let test_diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-04T05:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-04T20:00:00Z")?.to_utc(),
    );
    let functions = [CostFunctionImpl::TradingSimulation];

    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        info!("Train on {train_diapason:?}");
        let (mut all_values1, mut all_values2) = (Vec::new(), Vec::new());

        let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], train_diapason, Some(&db)).await?;
        convert_values(&order_books1, &order_books2, &mut all_values1, &mut all_values2);
        info!("Calculate std deviations");
        calculate_std_deviations(&mut all_values1);
        calculate_std_deviations(&mut all_values2);
        let events = merge_events(&all_values1, &all_values2, &[], &[]);
        info!("Total train events: {}", events.len());

        let parameters = functions.iter().map(|&func| {
            let params = calibrate_params(&events, func, DealDirection::Sell1Buy2);
            info!("{func:?} {params:?}");
            params
        }).collect::<Vec<_>>();

        info!("Test on {test_diapason:?}");
        all_values1.clear();
        all_values2.clear();
        let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], test_diapason, Some(&db)).await?;
        convert_values(&order_books1, &order_books2, &mut all_values1, &mut all_values2);
        info!("Calculate std deviations");
        calculate_std_deviations(&mut all_values1);
        calculate_std_deviations(&mut all_values2);

        let trades1 = db.get_trades(inst1_id, test_diapason).await?;
        let trades2 = db.get_trades(inst2_id, test_diapason).await?;
        let events = merge_events(&all_values1, &all_values2, &trades1, &trades2);
        info!("Total test events: {}", events.len());

        functions.iter().zip(parameters.iter()).for_each(|(&func, params)| {
            let result = run_simulation(&events, params);
            //let result = run_simulation_on_trades(&events, params, 10, false);
            info!("{func:?}. Deals {}. Income {}, outcome {}, commission {}, net profit {}",
            result.win + result.loss, result.income, result.outcome, result.commission, result.income - result.outcome - result.commission);
        });
    }

    Ok(())
}

fn convert_values(order_books1: &[OrderBook], order_books2: &[OrderBook], all_values1: &mut Vec<OrderBookValues>, all_values2: &mut Vec<OrderBookValues>) {
    all_values1.extend(order_books1.iter().filter_map(calculate_values));
    all_values2.extend(order_books2.iter().filter_map(calculate_values));
}

fn merge_events(values1: &[OrderBookValues], values2: &[OrderBookValues], trades1: &[Trade], trades2: &[Trade]) -> Vec<MarketEvent> {
    let mut events = Vec::with_capacity(values1.len() + values2.len());
    let (mut v_index1, mut v_index2, mut t_index1, mut t_index2) = (0, 0, 0, 0);
    while v_index1 < values1.len() || v_index2 < values2.len() || t_index1 < trades1.len() || t_index2 < trades2.len() {
        let mut cur_event = None;
        let mut cur_event_time = None;
        
        if let Some(v) = values1.get(v_index1) {
            let event = MarketEvent::OrderBook1(*v);
            let use_it = if let Some(event_time) = cur_event_time {
                v.time < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(v.time);
                cur_event = Some(event);
            }
        }

        if let Some(v) = values2.get(v_index2) {
            let event = MarketEvent::OrderBook2(*v);
            let use_it = if let Some(event_time) = cur_event_time {
                v.time < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(v.time);
                cur_event = Some(event);
            }
        }

        if let Some(t) = trades1.get(t_index1) {
            let event = MarketEvent::Deal1(*t);
            let use_it = if let Some(event_time) = cur_event_time {
                t.created < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(t.created);
                cur_event = Some(event);
            }
        }

        if let Some(t) = trades2.get(t_index2) {
            let event = MarketEvent::Deal2(*t);
            let use_it = if let Some(event_time) = cur_event_time {
                t.created < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(t.created);
                cur_event = Some(event);
            }
        }
        
        if let Some(event) = cur_event {
            match event {
                MarketEvent::OrderBook1(_) => v_index1 += 1,
                MarketEvent::OrderBook2(_) => v_index2 += 1,
                MarketEvent::Deal1(_) => t_index1 += 1,
                MarketEvent::Deal2(_) => t_index2 += 1,
            }
            events.push(event);
        } else {
            break;
        }
    }
    events
}

fn instrument_csv_header(ticker: &str) -> String {
    let mut header = format!("{ticker} time,{ticker} bid,{ticker} ask,{ticker} mid,{ticker} micro,{ticker} micro-mid");
    for levels in IMBALANCE_LEVELS {
        header.push_str(&format!(",{ticker} imbalance_{levels}"));
    }
    header
}

fn find_nearest_order_book(time: DateTime<Utc>, order_books: &[OrderBook], prev_index: usize) -> Option<usize> {
    let mut best_diff: Option<TimeDelta> = None;
    let mut best_index = None;

    let i_from = (prev_index as i32 - 10).max(0) as usize;
    let i_to = (prev_index + 10).min(order_books.len() - 1);
    for i in i_from..=i_to {
        let order_book = &order_books[i];
        let diff = (order_book.timestamp - time).abs();
        if let Some(cur_best) = best_diff {
            if diff < cur_best {
                best_diff = Some(diff);
                best_index = Some(i);
            }
        } else {
            best_diff = Some(diff);
            best_index = Some(i);
        }
    }

    best_index
}

#[derive(Copy, Clone, Debug)]
enum MarketEvent {
    OrderBook1(OrderBookValues),
    OrderBook2(OrderBookValues),
    Deal1(Trade),
    Deal2(Trade),
}

impl MarketEvent {
    pub fn event_datetime(&self) -> DateTime<Utc> {
        match self {
            MarketEvent::OrderBook1(order_book) => order_book.time,
            MarketEvent::OrderBook2(order_book) => order_book.time,
            MarketEvent::Deal1(deal) => deal.created,
            MarketEvent::Deal2(deal) => deal.created,
        }
    }
}

pub fn market_events_time_diapason(events: &[MarketEvent]) -> TimeDelta {
    events[events.len() - 1].event_datetime() - events[0].event_datetime()
}

#[derive(Copy, Clone, Debug)]
struct OrderBookValues {
    time: DateTime<Utc>,
    bid: f64,
    ask: f64,
    mid: f64,
    micro: f64,
    imbalances: [f64; IMBALANCE_LEVELS.len()],
    std_derivative: f64,
    trend: f64,
}

impl OrderBookValues {
    fn format_csv(&self) -> String {
        let mut str = format!("{},{},{},{},{},{}", self.time.to_rfc3339(), self.bid, self.ask, self.mid, self.micro, self.micro - self.mid);
        for imbalance in &self.imbalances {
            str.push_str(&format!(",{imbalance}"));
        }
        str
    }
}

fn calculate_values(order_book: &OrderBook) -> Option<OrderBookValues> {
    if order_book.asks.len() < 20 || order_book.bids.len() < 20 {
        return None;
    }

    let mut asks = order_book.asks.clone();
    asks.sort_by(|a, b| {a.price.partial_cmp(&b.price).unwrap()});

    let mut bids = order_book.bids.clone();
    bids.sort_by(|a, b| {b.price.partial_cmp(&a.price).unwrap()});

    let best_bid = bids[0].price.as_f64();
    let best_ask = asks[0].price.as_f64();
    let best_bid_volume = bids[0].size.as_f64();
    let best_ask_volume = asks[0].size.as_f64();
    let mid = (best_bid + best_ask) / 2.0;
    let micro = (best_ask * best_bid_volume + best_bid * best_ask_volume) / (best_bid_volume + best_ask_volume);

    let mut imbalances = [0.0; IMBALANCE_LEVELS.len()];
    for i in 0..IMBALANCE_LEVELS.len() {
        let levels = IMBALANCE_LEVELS[i];
        let (mut bids_sum, mut asks_sum) = (0.0, 0.0);
        for level in 0..levels {
            if let Some(bid) = bids.get(level) {
                bids_sum += bid.size.as_f64();
            }
            if let Some(ask) = asks.get(level) {
                asks_sum += ask.size.as_f64();
            }
        }
        imbalances[i] = (bids_sum - asks_sum) / (bids_sum + asks_sum);
    }

    Some(OrderBookValues {
        time: order_book.timestamp,
        bid: best_bid,
        ask: best_ask,
        mid,
        micro,
        imbalances,
        std_derivative: 0.0,
        trend: 0.0,
    })
}

const STD_DEVIATION_WINDOW_SECONDS: i64 = 15 * 60;
const TREND_TAU_SECONDS: f64 = 30.0;

fn calculate_std_deviations(values: &mut [OrderBookValues]) {
    let mut window_mids = Vec::new();
    let mut window_derivatives = Vec::new();
    let mut window_times = Vec::<DateTime<Utc>>::new();

    for (i, v) in values.iter_mut().enumerate() {
        let window_start = v.time - TimeDelta::seconds(STD_DEVIATION_WINDOW_SECONDS);
        while !window_times.is_empty() && window_times[0] < window_start {
            window_times.remove(0);
            window_derivatives.remove(0);
            window_mids.remove(0);
        }

        let mut prev_index = window_mids.len() as i32 - 1;
        while prev_index >= 0 && window_times[prev_index as usize] == v.time {
            prev_index -= 1;
        }

        let derivative: f64 = if prev_index >= 0 {
            (v.mid - window_mids[prev_index as usize]) / (v.time.timestamp_millis() - window_times[prev_index as usize].timestamp_millis()) as f64
        } else { 0.0 };

        if derivative.abs() > f64::MIN_POSITIVE {
            let d = if let Some(std) = standard_deviation(&window_derivatives) {
                if std > 0.0000001 {
                    derivative / std
                } else {
                    if derivative > 0.0 { 100.0 } else { -100.0 }
                }
            } else {
                0.0
            };
            if !d.is_nan() {
                v.std_derivative = d;
            }
        }

        window_mids.push(v.mid);
        window_derivatives.push(derivative);
        window_times.push(v.time);
    }
}

fn calculate_trend(values: &mut [OrderBookValues]) {
    let mut trend = 0.0;
    let mut prev_time = Option::<DateTime<Utc>>::None;

    for value in values.iter_mut() {
        if let Some(cur_prev_time) = prev_time {
            //if value.time > cur_prev_time {
                let dt = (value.time - cur_prev_time).as_seconds_f64();
                let alpha = 1.0 - (-dt / TREND_TAU_SECONDS).exp();
                trend += alpha * (value.std_derivative - trend);
                value.trend = trend;
                prev_time = Some(value.time);
            //} else {
            //    value.trend = trend;
            //}
        } else {
            trend = value.std_derivative;
            value.trend = trend;
            prev_time = Some(value.time);
        }
    }
}