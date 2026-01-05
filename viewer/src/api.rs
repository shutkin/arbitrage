#[cfg(feature = "ssr")]
use chrono::{DateTime, TimeDelta, Duration, Utc};
use chrono::FixedOffset;
#[cfg(feature = "ssr")]
use rust_decimal::Decimal;
#[cfg(feature = "ssr")]
use rust_decimal::prelude::ToPrimitive;
#[cfg(feature = "ssr")]
use model::common::{CommonError, TimeDiapason};
#[cfg(feature = "ssr")]
use model::OrderData;
#[cfg(feature = "ssr")]
use model::utils::{best_price, filter_order_data};
#[cfg(feature = "ssr")]
use crate::client_model::{PricesParams, ClientInstrument};
#[cfg(feature = "ssr")]
use log::{info};
use model::utils::{calculate_delta_buy, calculate_delta_sell};
use crate::client_model::{DeltaParams, MainParams, PricesResponse, TimedValues, NAN_VALUE};

const PRICES_RESOLUTION: usize = 1600;

#[derive(Clone)]
#[cfg(feature = "ssr")]
pub struct API {
    db: db::Db,
}

#[cfg(feature = "ssr")]
impl API {
    pub async fn new(db_url: &str) -> Result<Self, CommonError> {
        let db = db::Db::new(db_url).await?;
        Ok(API { db })
    }

    pub async fn get_instruments(&self) -> Result<Vec<ClientInstrument>, CommonError> {
        Ok(self.db.get_instruments(true).await?.iter()
            .map(|instrument| ClientInstrument {
                id: instrument.id.unwrap_or_default(),
                symbol: instrument.symbol.clone(),
            }).collect())
    }

    pub async fn get_deltas(&self, main_params: MainParams, params: DeltaParams) -> Result<Vec<TimedValues>, CommonError> {
        let offset = main_params.date_time.offset();
        let diapason = TimeDiapason::new(
            main_params.date_time.to_utc() - Duration::minutes(main_params.interval_minutes as i64),
            main_params.date_time.to_utc());
        let order_books1 = self.db.get_order_books(Some(params.instrument1), diapason, db::QueryAsksOrBids::BOTH).await?;
        let order_books2 = self.db.get_order_books(Some(params.instrument2), diapason, db::QueryAsksOrBids::BOTH).await?;
        if order_books1.is_empty() || order_books2.is_empty() {
            return Err("No data".into());
        }

        let min_volume = Decimal::ONE_THOUSAND;
        let mut deltas_sell = Vec::new();
        let mut deltas_buy = Vec::new();
        let (mut index1, mut index2) = (0, 0);
        while index1 < order_books1.len() - 1 && index2 < order_books2.len() - 1 {
            let (data1, data2) = (&order_books1[index1], &order_books2[index2]);
            
            if let Some(delta_sell) = calculate_delta_sell(data1, data2, min_volume) {
                deltas_sell.push((data1.timestamp, delta_sell));
            }
            if let Some(delta_buy) = calculate_delta_buy(data1, data2, min_volume) {
                deltas_buy.push((data1.timestamp, delta_buy));
            }
            
            if data1.timestamp < data2.timestamp {
                index1 += 1;
            } else {
                index2 += 1;
            }
        }
        Ok(Self::group_data(&[deltas_sell, deltas_buy], offset))
    }

    pub async fn get_prices(&self, main_params: MainParams, params: PricesParams) -> Result<PricesResponse, CommonError> {
        let offset = main_params.date_time.offset();
        let diapason = TimeDiapason::new(
            main_params.date_time.to_utc() - Duration::minutes(main_params.interval_minutes as i64),
            main_params.date_time.to_utc());
        let mut data = Vec::with_capacity(params.instrument_ids_asks.len() + params.instrument_ids_bids.len());
        for instrument_id in &params.instrument_ids_asks {
            let order_books = self.db.get_order_books(Some(*instrument_id), diapason, db::QueryAsksOrBids::ASKS).await?;
            let mut series = Vec::with_capacity(order_books.len());
            for order_book in order_books {
                if let Some(ask) = best_price(&filter_order_data(&order_book.asks), true) {
                    series.push((order_book.timestamp, ask));
                }
            }
            if !series.is_empty() {
                data.push(series);
            }
        }
        for instrument_id in &params.instrument_ids_bids {
            let order_books = self.db.get_order_books(Some(*instrument_id), diapason, db::QueryAsksOrBids::BIDS).await?;
            let mut series = Vec::with_capacity(order_books.len());
            for order_book in order_books {
                if let Some(bid) = best_price(&filter_order_data(&order_book.bids), false) {
                    series.push((order_book.timestamp, bid));
                }
            }
            if !series.is_empty() {
                data.push(series);
            }
        }
        if data.is_empty() {
            Err("No data".into())
        } else {
            let started = Utc::now();
            let data = Self::group_data(&data, offset);
            info!("Grouping takes {}", Utc::now() - started);
            Ok(PricesResponse {
                names: self.instruments_names(&params).await?,
                data,
            })
        }
    }

    async fn instruments_names(&self, params: &PricesParams) -> Result<Vec<String>, CommonError> {
        let instruments = self.db.get_instruments(false).await?;

        let mut result = Vec::new();
        for instrument_id in &params.instrument_ids_asks {
            if let Some(instrument) = instruments.iter()
                .find(|inst| inst.id == Some(*instrument_id)) {
                result.push(format!("{} ask", instrument.symbol));
            }
        }
        for instrument_id in &params.instrument_ids_bids {
            if let Some(instrument) = instruments.iter()
                .find(|inst| inst.id == Some(*instrument_id)) {
                result.push(format!("{} bid", instrument.symbol));
            }
        }
        Ok(result)
    }

    fn group_data(data: &[Vec<(DateTime<Utc>, OrderData)>], offset: &FixedOffset) -> Vec<TimedValues> {
        let start_timestamp = data.iter()
            .map(|v| v.first().unwrap().0).max().unwrap();
        let start = start_timestamp.timestamp_millis();
        let finish = data.iter()
            .map(|v| v.last().unwrap().0).min().unwrap().timestamp_millis();
        let length = finish - start;
        let channels = data.len();

        let mut groups = vec![vec![(Decimal::ZERO, Decimal::ZERO); channels]; PRICES_RESOLUTION];
        for (channel_index, series) in data.iter().enumerate() {
            for (timestamp, value) in series {
                let time = timestamp.timestamp_millis() - start;
                let group_index = (time * (PRICES_RESOLUTION + 1) as i64 / length - 2)
                    .max(0).min(PRICES_RESOLUTION as i64 - 1) as usize;
                let (sum_price, sum_size) = groups[group_index][channel_index];
                groups[group_index][channel_index] = (value.price * value.size + sum_price, value.size + sum_size);
            }
        }

        let mut result = Vec::with_capacity(PRICES_RESOLUTION);
        for (group_index, channels_values) in groups.into_iter().enumerate() {
            let time = (group_index + 1) as i64 * length / PRICES_RESOLUTION as i64;
            let timestamp = (start_timestamp + TimeDelta::milliseconds(time)).with_timezone(offset);

            let values = channels_values.iter().enumerate()
                .map(|(channel_index, (sum_price, sum_size))| {
                    if *sum_size != Decimal::ZERO {
                        (sum_price / sum_size).to_f64().unwrap()
                    } else {
                        //prev_values[channel_index]
                        NAN_VALUE * 2.0
                    }
                }).collect::<Vec<_>>();
            result.push(TimedValues { timestamp, values });
        }
        result
    }
}