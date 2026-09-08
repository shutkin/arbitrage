mod order_book_cache;
mod signal_params;
mod math_utils;
mod simulation;
mod stats_collector;
mod deal;

use crate::math_utils::std_derivative;
use crate::signal_params::{IMBALANCE_LEVELS, SignalParams, SignalParamsDir, calibrate_params};
use crate::simulation::run_simulation_on_trades;
use chrono::{DateTime, Duration, TimeDelta, Utc};
use db::{Db, QueryAsksOrBids};
use log::info;
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::{Instrument, OrderBook, Trade};
use simplelog::{LevelFilter, SimpleLogger};
use crate::stats_collector::{signal_after_deal, threshold_horizon_probabilities};

const COMMISSION_RATIO: f64 = 0.015 / 100.0;

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
async fn _main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        let test_diapason = TimeDiapason::new(
            DateTime::parse_from_rfc3339("2026-09-04T10:00:00Z")?.to_utc(),
            DateTime::parse_from_rfc3339("2026-09-04T20:00:00Z")?.to_utc(),
        );
        let trades1 = db.get_trades(inst1_id, test_diapason).await?;
        let trades2 = db.get_trades(inst2_id, test_diapason).await?;
        let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], test_diapason, Some(&db)).await?;
        let mut values1 = order_books1.iter().filter_map(calculate_values).collect::<Vec<_>>();
        let mut all_values2 = order_books2.iter().filter_map(calculate_values).collect::<Vec<_>>();
        info!("Calculate std deviations");
        calculate_std_deviations(&mut values1);
        calculate_std_deviations(&mut all_values2);

        info!(
            "Merge {} values1, {} values2, {} deals1, {} deals2",
            values1.len(), all_values2.len(), trades1.len(), trades2.len()
        );
        let events = merge_events(&values1, &all_values2, &trades1, &trades2);
        info!("Total events: {}", events.len());
/*
        // Unknown old params, data from 03.09.2026
        let params = SignalParams { signal_threshold: 0.0, hold_ms: 250, up: Some(SignalParamsDir { alpha: 0.15453738797718342, derivative1_weight: 1.1629761023166723, derivative2_weight: 0.32481462380662857, imbalance1_weights: [-0.09802006258516288, -0.04839597102546962, -0.0549509873464885, -0.15984030244392058], imbalance2_weights: [-0.06759615045675671, -0.049698124522988454, 0.061509796401540175, 0.06838655861917534] }), down: Some(SignalParamsDir { alpha: 0.12809776356042107, derivative1_weight: 1.2524761056539522, derivative2_weight: 1.0711123993550387, imbalance1_weights: [-0.08348685530090004, -0.19510914374521984, -0.22190776923813244, -0.004042762088653724], imbalance2_weights: [-0.05784155826200306, -0.01112889975677297, -0.06887535463793135, -0.12454322697759054] }) };

        let table = threshold_horizon_probabilities(&events, params);
        info!("threshold_horizon_probabilities:\n{}", table);

        let table = signal_after_deal(&events, params);
        info!("signal_after_deal:\n{}", table);

        let result = run_simulation_on_trades(&events, &params, 10, false);
        info!("Simulation profit on {} deals: {:?}", result.win + result.loss, result.income - result.outcome - result.commission);

        // Actual params with no commission, data from 03.09.2026
        let params = SignalParams { hold_ms: 250, signal_threshold: 0.0, up: Some(SignalParamsDir { alpha: -0.20492515627874663, derivative1_weight: 0.6201758022694368, derivative2_weight: -0.8719143683779658, imbalance1_weights: [0.16343937671236203, 0.03371568907026376, 0.13219643355353505, -0.3484609745459927], imbalance2_weights: [0.043650570830156145, 0.11762212484538968, 0.20559270007913955, 0.053242022584380516] }), down: Some(SignalParamsDir { alpha: -0.1605505448272192, derivative1_weight: 1.8930396501868383, derivative2_weight: -0.28244783119020245, imbalance1_weights: [0.1542439073301614, -0.09103354910836434, -0.39035293689465383, -0.011130502635283455], imbalance2_weights: [-0.03357607437194851, -0.018782747678162147, -0.031572841035818325, 0.02193815722363937] }) };
        let result = run_simulation_on_trades(&events, &params, 10, false);
        info!("Simulation profit on {} deals: {:?}", result.win + result.loss, result.income - result.outcome - result.commission);

        // Actual params with commission, data from 03.09.2026
        let params = SignalParams { hold_ms: 250, signal_threshold: 0.0, up: Some(SignalParamsDir { alpha: -12.872754435045326, derivative1_weight: 0.193634494133364, derivative2_weight: -0.1457796752877215, imbalance1_weights: [0.18149400301128937, 0.3541486748959699, 0.2430203348794835, 0.15951349406028242], imbalance2_weights: [0.010143189611607064, 0.3292634068999326, 0.08917263792538223, 0.02371948518984832] }), down: Some(SignalParamsDir { alpha: -12.64299798348242, derivative1_weight: 0.36230450821391014, derivative2_weight: 0.03658025805441441, imbalance1_weights: [0.18631803136754216, 0.11059152171389611, 0.25328099551332484, 0.3910631898521122], imbalance2_weights: [-0.049288649334782476, 0.14452138108540152, 0.0403198164700859, 0.04841766623946943] }) };
        let result = run_simulation_on_trades(&events, &params, 10, false);
        info!("Simulation profit on {} deals: {:?}", result.win + result.loss, result.income - result.outcome - result.commission);
*/    }
    Ok(())
}

#[tokio::main]
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let test_diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-04T05:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-04T15:00:00Z")?.to_utc(),
    );
    let train_diapason = TimeDiapason::new(
        DateTime::parse_from_rfc3339("2026-09-03T13:00:00Z")?.to_utc(),
        DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")?.to_utc(),
    );

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

        let params = calibrate_params(&merge_events(&all_values1, &all_values2, &[], &[]));
        info!("{params:?}");

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

        let result = run_simulation_on_trades(&events, &params, 10, false);
        info!("Deals {}. Income {}, outcome {}, commission {}, net {}",
            result.win + result.loss, result.income, result.outcome, result.commission, result.income - result.outcome - result.commission);
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

#[derive(Copy, Clone, Debug)]
struct OrderBookValues {
    time: DateTime<Utc>,
    bid: f64,
    ask: f64,
    mid: f64,
    micro: f64,
    imbalances: [f64; IMBALANCE_LEVELS.len()],
    std_derivative: f64,
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
    })
}

fn calculate_std_deviations(values: &mut [OrderBookValues]) {
    for i in 32..values.len() {
        let d = std_derivative(values, i);
        values[i].std_derivative = d;
    }
}