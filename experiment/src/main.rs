mod order_book_cache;
mod signal_params;
mod math_utils;
mod simulation;

extern crate core;

use crate::math_utils::std_derivative;
use crate::signal_params::{IMBALANCE_LEVELS, calibrate_params};
use crate::simulation::run_simulation;
use chrono::{DateTime, Duration, TimeDelta, Utc};
use db::{Db, QueryAsksOrBids};
use log::info;
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::{Instrument, OrderBook};
use simplelog::{LevelFilter, SimpleLogger};

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
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let start = DateTime::parse_from_rfc3339("2026-09-04T08:00:00Z")?.to_utc();
    let finish = DateTime::parse_from_rfc3339("2026-09-04T19:00:00Z")?.to_utc();
    let mut chunk_start = start - Duration::milliseconds(1);

    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        //let mut csv = Vec::new();
        //let header1 = instrument_csv_header(tickers[0]);
        //let header2 = instrument_csv_header(tickers[1]);
        //csv.push(format!("{header1},{header2}"));
        let mut prev_log_time = start;
        let (mut all_values1, mut all_values2) = (Vec::new(), Vec::new());
        while chunk_start < finish - Duration::hours(1) {
            let chunk_end = (chunk_start + Duration::minutes(120)).min(finish);
            let diapason = TimeDiapason::new(chunk_start + Duration::milliseconds(1), chunk_end);
            let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], diapason, Some(&db)).await?;
            let mut prev_index2 = 0;
            for order_book1 in &order_books1 {
                if let Some(index2) = find_nearest_order_book(order_book1.timestamp, &order_books2, prev_index2) {
                    if let Some(values1) = calculate_values(order_book1) &&
                        let Some(values2) = calculate_values(&order_books2[index2]) {
                        all_values1.push(values1);
                        all_values2.push(values2);
                    }
                    //csv.push(format!("{},{}", values1.format_csv(), values2.format_csv()));
                    prev_index2 = index2;
                }
                if order_book1.timestamp > prev_log_time + TimeDelta::minutes(5) {
                    info!("{}", order_book1.timestamp);
                    prev_log_time = order_book1.timestamp;
                }
            }
            chunk_start = chunk_end;
        }
        calculate_std_deviations(&mut all_values1);
        calculate_std_deviations(&mut all_values2);
        //std::fs::write("results.csv", csv.join("\n"))?;

        let params = calibrate_params(&all_values1, &all_values2);
        info!("{params:?}");

        all_values1.clear();
        all_values2.clear();
        let test_diapason = TimeDiapason::new(
            DateTime::parse_from_rfc3339("2026-09-05T08:00:00Z")?.to_utc(),
            DateTime::parse_from_rfc3339("2026-09-05T11:00:00Z")?.to_utc());
        let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], test_diapason, Some(&db)).await?;
        let mut prev_index2 = 0;
        for order_book1 in &order_books1 {
            if let Some(index2) = find_nearest_order_book(order_book1.timestamp, &order_books2, prev_index2) {
                if let Some(values1) = calculate_values(order_book1) &&
                    let Some(values2) = calculate_values(&order_books2[index2]) {
                    all_values1.push(values1);
                    all_values2.push(values2);
                }
                prev_index2 = index2;
            }
            if order_book1.timestamp > prev_log_time + TimeDelta::minutes(5) {
                info!("{}", order_book1.timestamp);
                prev_log_time = order_book1.timestamp;
            }
        }
        calculate_std_deviations(&mut all_values1);
        calculate_std_deviations(&mut all_values2);

        let result = run_simulation(&all_values1, &all_values2, &params);
        info!("Wins {}, losses {}", result.win, result.loss);
        info!("Income {}, outcome {}, commission {}, profit {}", result.income, result.outcome, result.commission, result.income - result.outcome - result.commission);
    }

    Ok(())
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