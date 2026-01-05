use chrono::{DateTime, Utc};
use log::{debug, info};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Deserialize;
use model::common::{CommonError, TimeDiapason};
use model::{Instrument, OrderBook, OrderData};

const REST_URL: &str = "https://api.bybit.com";
const INSTRUMENTS_ENDPOINT: &str = "/v5/market/instruments-info";
const KLINE_ENDPOINT: &str = "/v5/market/kline";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitResponse<T> {
    ret_code: i32,
    ret_msg: String,
    result: Option<T>,
}

#[derive(Deserialize)]
struct ResponseKLine {
    symbol: String,
    category: String,
    list: Vec<Vec<Decimal>>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseInstruments {
    category: String,
    next_page_cursor: Option<String>,
    list: Vec<ResponseInstrument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseInstrument {
    symbol: String,
    status: String,
    base_coin: String,
    quote_coin: String,
}

pub async fn get_kline(symbol: &str, diapason: TimeDiapason) -> Result<Vec<OrderBook>, CommonError> {
    let url = format!("{REST_URL}{KLINE_ENDPOINT}?category=linear&symbol={symbol}&interval=1&start={}&end={}&limit=1000",
        diapason.from.timestamp_millis(), diapason.to.timestamp_millis());
    debug!("GET {url}");
    let response = reqwest::get(&url).await?;
    let text = response.text().await?;
    let response = serde_json::from_str::<BybitResponse<ResponseKLine>>(&text)?;
    debug!("Response: {} {}", response.ret_code, response.ret_msg);
    let list = response.result.ok_or::<CommonError>("No response".into())?.list;
    let result = list.into_iter()
        .filter(|list| list.len() > 5 && !list[5].is_zero())
        .map(|list| {
            let timestamp_ms = list[0].to_i64().unwrap();
            let timestamp = DateTime::from_timestamp_millis(timestamp_ms).unwrap();
            let highest = list[2];
            let lowest = list[3];
            let volume = Decimal::ONE_HUNDRED;
            OrderBook {
                id: None,
                timestamp,
                bids: vec![OrderData { price: lowest, size: volume }],
                asks: vec![OrderData { price: highest, size: volume }],
            }
        }).collect::<Vec<_>>();
    if !result.is_empty() {
        info!("Collected {} prices from {} to {}", result.len(), result[0].timestamp, result[result.len() - 1].timestamp);
    }
    Ok(result)
}

pub async fn get_instrument_info() -> Result<Vec<Instrument>, CommonError> {
    let mut result = Vec::new();

    let response = reqwest::get(&format!("{REST_URL}{INSTRUMENTS_ENDPOINT}/?category=linear")).await?;
    let mut next_cursor = handle_instruments_resp(&mut result, response).await?;
    while let Some(cursor) = &next_cursor {
        let response = reqwest::get(&format!("{REST_URL}{INSTRUMENTS_ENDPOINT}/?category=linear&cursor={cursor}")).await?;
        next_cursor = handle_instruments_resp(&mut result, response).await?;
    }

    Ok(result)
}

async fn handle_instruments_resp(result: &mut Vec<Instrument>, response: reqwest::Response) -> Result<Option<String>, CommonError> {
    let text = response.text().await?;
    let response = serde_json::from_str::<BybitResponse<ResponseInstruments>>(&text)?;
    info!("Response: {} {}", response.ret_code, response.ret_msg);

    if let Some(instruments) = &response.result {
        debug!("Category: {}, Next page cursor: {:?}", instruments.category, instruments.next_page_cursor);

        instruments.list.iter().for_each(|instrument| {
            result.push(Instrument {
                id: None,
                symbol: instrument.symbol.clone(),
                status: instrument.status.clone(),
                base_coin: instrument.base_coin.clone(),
                quote_coin: instrument.quote_coin.clone(),
            })
        });
        let cursor = if let Some(next_cursor) = &instruments.next_page_cursor {
            if next_cursor.is_empty() {
                None
            } else {
                Some(String::from(next_cursor))
            }
        } else {
            None
        };
        Ok(cursor)
    } else {
        Ok(None)
    }
}