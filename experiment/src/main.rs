mod pair_trader;
mod order_book_cache;
mod perfect_trader;

extern crate core;

use std::fs::{File, OpenOptions};
use std::io::Write;
use chrono::{DateTime, Duration, Utc};
use db::{Db, QueryAsksOrBids};
use log::info;
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::{Instrument, OrderBook};
use rust_decimal::Decimal;
use simplelog::{LevelFilter, SimpleLogger};
use bybit::rest_api::get_kline;
use crate::pair_trader::{PairTrader, PairTraderConfig};
use crate::perfect_trader::PerfectTrader;

fn find_instrument_id(instruments: &[Instrument], symbol: &str) -> Option<i16> {
    instruments
        .iter()
        .find(|instrument| instrument.symbol == symbol)
        .and_then(|instrument| instrument.id)
}

async fn get_order_books(symbols: &[&str], ids: &[i16], diapason: TimeDiapason, is_api: bool, db: Option<&Db>)
    -> Result<(Vec<OrderBook>, Vec<OrderBook>), CommonError> {
    if let (Some(order_books1), Some(order_books2)) =
        (order_book_cache::read(symbols[0], diapason, !is_api)?,
         order_book_cache::read(symbols[1], diapason, !is_api)?) {
        Ok((order_books1, order_books2))
    } else {
        let (order_books1, order_books2) = if is_api {
            (get_kline(symbols[0], diapason).await?, get_kline(symbols[1], diapason).await?)
        } else {
            (db.unwrap().get_order_books(Some(ids[0]), diapason, QueryAsksOrBids::BOTH).await?,
             db.unwrap().get_order_books(Some(ids[1]), diapason, QueryAsksOrBids::BOTH).await?)
        };
        order_book_cache::write(symbols[0], diapason, &order_books1, !is_api)?;
        order_book_cache::write(symbols[1], diapason, &order_books2, !is_api)?;
        Ok((order_books1, order_books2))
    }
}

async fn _main() -> EmptyResult {
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();

    let pairs = [
        ["BTCPERP", "BTCUSDT-28NOV25"],
        ["BTCPERP", "BTCUSDT-26DEC25"],
        ["BTCPERP", "BTCUSDT-27MAR26"],
        ["BTCPERP", "BTCUSDT-26JUN26"],
        ["BTCPERP", "BTCUSDT-25SEP26"],
        ["BTCUSDT-28NOV25", "BTCUSDT-26DEC25"],
        ["BTCUSDT-28NOV25", "BTCUSDT-27MAR26"],
        ["BTCUSDT-28NOV25", "BTCUSDT-26JUN26"],
        ["BTCUSDT-28NOV25", "BTCUSDT-25SEP26"],
        ["BTCUSDT-26DEC25", "BTCUSDT-27MAR26"],
        ["BTCUSDT-26DEC25", "BTCUSDT-26JUN26"],
        ["BTCUSDT-26DEC25", "BTCUSDT-25SEP26"],
        ["BTCUSDT-27MAR26", "BTCUSDT-26JUN26"],
        ["BTCUSDT-27MAR26", "BTCUSDT-25SEP26"],
        ["BTCUSDT-26JUN26", "BTCUSDT-25SEP26"],

        ["ETHPERP", "ETHUSDT-28NOV25"],
        ["ETHPERP", "ETHUSDT-26DEC25"],
        ["ETHPERP", "ETHUSDT-27MAR26"],
        ["ETHPERP", "ETHUSDT-26JUN26"],
        ["ETHPERP", "ETHUSDT-25SEP26"],
        ["ETHUSDT-28NOV25", "ETHUSDT-26DEC25"],
        ["ETHUSDT-28NOV25", "ETHUSDT-27MAR26"],
        ["ETHUSDT-28NOV25", "ETHUSDT-26JUN26"],
        ["ETHUSDT-28NOV25", "ETHUSDT-25SEP26"],
        ["ETHUSDT-26DEC25", "ETHUSDT-27MAR26"],
        ["ETHUSDT-26DEC25", "ETHUSDT-26JUN26"],
        ["ETHUSDT-26DEC25", "ETHUSDT-25SEP26"],
        ["ETHUSDT-27MAR26", "ETHUSDT-26JUN26"],
        ["ETHUSDT-27MAR26", "ETHUSDT-25SEP26"],
        ["ETHUSDT-26JUN26", "ETHUSDT-25SEP26"],
    ];
    let start = DateTime::parse_from_rfc3339("2025-10-14T00:00:00Z").unwrap().to_utc();
    let finish = DateTime::parse_from_rfc3339("2025-11-02T08:00:00Z").unwrap().to_utc();

    let mut result = Vec::new();
    for pair in pairs {
        let mut chunk_start = start - Duration::milliseconds(1);
        let mut trader = PerfectTrader::default();
        let mut total_profit = Decimal::ZERO;
        while chunk_start < finish - Duration::hours(1) {
            let chunk_end = (chunk_start + Duration::minutes(120)).min(finish);
            let diapason = TimeDiapason::new(chunk_start + Duration::milliseconds(1), chunk_end);
            let (order_books1, order_books2) = get_order_books(&pair, &[0, 0], diapason, true, None).await?;
            let (mut index1, mut index2) = (0, 0);
            while (index1 as i32) < order_books1.len() as i32 - 1 && (index2 as i32) < order_books2.len() as i32 - 1 {
                let (data1, data2) = (&order_books1[index1], &order_books2[index2]);
                if let Some(deal) = if data1.timestamp > data2.timestamp {
                    index2 += 1;
                    trader.trade(data1, true)
                } else {
                    index1 += 1;
                    trader.trade(data2, false)
                } {
                    total_profit += deal.profit();
                }
            }
            chunk_start = chunk_end;
        }
        result.push((pair, total_profit));
        info!("Pair {:?}: {}", pair, total_profit);
    }

    result.sort_by_key(|(_, profit)| *profit);
    for i in 0..result.len() {
        let (pair, profit) = result[result.len() - i - 1];
        info!("{:?}: {}", pair, profit);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    //let instruments = ["BTCUSDT-26DEC25", "BTCUSDT-27MAR26"];
    let instruments = ["ETHUSDT-26DEC25", "ETHUSDT-26JUN26"];
    //let start = DateTime::parse_from_rfc3339("2025-10-14T00:00:00Z").unwrap().to_utc();
    //let finish = DateTime::parse_from_rfc3339("2025-11-02T08:00:00Z").unwrap().to_utc();
    let start = DateTime::parse_from_rfc3339("2025-11-03T13:00:00Z").unwrap().to_utc();
    let finish = Utc::now();
    let mut chunk_start = start - Duration::milliseconds(1);

    let filename = format!("trader_{}.csv", Utc::now().timestamp());
    let mut file = File::create_new(&filename)?;
    file.write_all("open,close,amount1,amount2,buy1,buy2,sell1,sell2,profit,misc,misc2\n".as_bytes())?;
    file.sync_all()?;
    let mut total_profit = Decimal::ZERO;
    let mut trader = PairTrader::new(PairTraderConfig {
        deal_amount: Decimal::from(500),
        delta_to_median_ratio: 0.94 ,
        medians_ratio: 0.975,
        median_interval_minutes: 30,
        min_deal_interval_millis: 200,
        median_lifetime_seconds: 5,
        calc_median: true,
    });

    let mut trader = PerfectTrader::default();
    
    let bybit_api_source = false;

    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, instruments[0]),
        find_instrument_id(&all_instruments, instruments[1]),
    ) {
        while chunk_start < finish - Duration::hours(1) {
            let chunk_end = (chunk_start + Duration::minutes(120)).min(finish);
            let diapason = TimeDiapason::new(chunk_start + Duration::milliseconds(1), chunk_end);
            let (order_books1, order_books2) = get_order_books(&instruments, &[inst1_id, inst2_id], diapason, bybit_api_source, Some(&db)).await?;
            let (mut index1, mut index2) = (0, 0);
            while (index1 as i32) < order_books1.len() as i32 - 1 && (index2 as i32) < order_books2.len() as i32 - 1 {
                let (data1, data2) = (&order_books1[index1], &order_books2[index2]);
                if let Some(deal) = if data1.timestamp > data2.timestamp {
                    index2 += 1;
                    trader.trade(data1, true)
                } else {
                    index1 += 1;
                    trader.trade(data2, false)
                } {
                    let mut file = OpenOptions::new().append(true).open(&filename)?;
                    let line = format!("{},{},{},{},{},{},{},{},{},{},{}\n",
                                       deal.open_time, deal.close_time,
                                       deal.amount[0], deal.amount[1],
                                       deal.buy_price[0], deal.buy_price[1],
                                       deal.sell_price[0], deal.sell_price[1],
                                       deal.profit(), deal.misc, deal.misc2,
                    );
                    file.write_all(line.as_bytes())?;
                    total_profit += deal.profit();
                }
            }

            chunk_start = chunk_end;
        }

        info!("Total profit: {total_profit}");
    }

    Ok(())
}
