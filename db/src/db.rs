use chrono::{DateTime, Utc};
use log::{debug, info};
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::utils::handle_delta;
use model::{Instrument, OrderBook, OrderData, OrderDataDelta};
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub enum QueryAsksOrBids {
    ASKS,
    BIDS,
    BOTH,
}

impl QueryAsksOrBids {
    fn is_asks(&self) -> bool {
        match self {
            QueryAsksOrBids::ASKS => true,
            QueryAsksOrBids::BIDS => false,
            QueryAsksOrBids::BOTH => true,
        }
    }

    fn is_bids(&self) -> bool {
        match self {
            QueryAsksOrBids::ASKS => false,
            QueryAsksOrBids::BIDS => true,
            QueryAsksOrBids::BOTH => true,
        }
    }
}

#[derive(Clone)]
struct Prices {
    prices_map: Arc<RwLock<HashMap<Decimal, i32>>>,
}

impl Prices {
    fn new() -> Self {
        Self {
            prices_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn init(&self, pool: &Pool<Postgres>) -> EmptyResult {
        let rows = sqlx::query("SELECT id, price FROM price")
            .fetch_all(pool)
            .await?;
        let mut lock = self.prices_map.write().await;
        rows.iter().for_each(|row| {
            lock.insert(row.get(1), row.get(0));
        });
        info!("Prices init with {} values", lock.len());
        Ok(())
    }

    async fn get_id(&self, price: Decimal, pool: &Pool<Postgres>) -> Result<i32, CommonError> {
        if let Some(id) = self.prices_map.read().await.get(&price) {
            Ok(*id)
        } else {
            info!("Insert new price value {price}");
            let mut lock = self.prices_map.write().await;
            let id_row = sqlx::query("INSERT INTO price (price) VALUES ($1) RETURNING id")
                .bind(price)
                .fetch_one(pool)
                .await?;
            let id = id_row.get(0);
            lock.insert(price, id);
            Ok(id)
        }
    }

    async fn get_prices(&self, map: &mut HashMap<i32, Decimal>) {
        map.clear();
        let lock = self.prices_map.read().await;
        for (price, id) in lock.iter() {
            map.insert(*id, *price);
        }
    }
}

struct DbDictionary {
    table_name: String,
    dictionary: HashMap<String, i16>,
}

impl DbDictionary {
    async fn query(pool: &Pool<Postgres>, table_name: &str) -> Result<Self, CommonError> {
        let rows = sqlx::query(&format!("SELECT code, name FROM {table_name}"))
            .fetch_all(pool)
            .await?;
        let mut dictionary = HashMap::<String, i16>::with_capacity(rows.len());
        rows.iter().for_each(|row| {
            let code = row.get(0);
            dictionary.insert(row.get(1), code);
        });
        Ok(Self {
            table_name: String::from(table_name),
            dictionary,
        })
    }

    async fn get_code(&mut self, pool: &Pool<Postgres>, value: &str) -> Result<i16, CommonError> {
        if let Some(code) = self.dictionary.get(value) {
            Ok(*code)
        } else {
            let new_code = self
                .dictionary
                .values()
                .max()
                .map(|code| *code + 1)
                .unwrap_or(0);
            info!(
                "Insert new value '{value}' with code {new_code} into {}",
                self.table_name
            );
            sqlx::query(&format!(
                "INSERT INTO {} (code, name) VALUES ($1, $2)",
                self.table_name
            ))
            .bind(new_code)
            .bind(String::from(value))
            .execute(pool)
            .await?;
            self.dictionary.insert(String::from(value), new_code);
            Ok(new_code)
        }
    }
}
#[derive(Clone)]
pub struct Db {
    pool: Pool<Postgres>,
    prices: Prices,
}

impl Db {
    pub async fn new(url: &str) -> Result<Self, CommonError> {
        let pool = PgPoolOptions::new().max_connections(8).connect(url).await?;
        let prices = Prices::new();
        prices.init(&pool).await?;
        Ok(Self { pool, prices })
    }

    pub async fn get_instruments(&self, last_hour: bool) -> Result<Vec<Instrument>, CommonError> {
        let rows = sqlx::query(if last_hour {
            "SELECT i.id, i.symbol, s.name, bc.name, qc.name FROM instrument i JOIN instrument_status s on i.status_code = s.code JOIN coin bc ON i.base_coin_code = bc.code JOIN coin qc ON i.quote_coin_code = qc.code WHERE i.id in (SELECT DISTINCT instrument_id FROM order_book WHERE created > now() - interval '1 hour');"
        } else {
            "SELECT i.id, i.symbol, s.name, bc.name, qc.name FROM instrument i JOIN instrument_status s on i.status_code = s.code JOIN coin bc ON i.base_coin_code = bc.code JOIN coin qc ON i.quote_coin_code = qc.code"
        })
            .fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|row| Instrument {
                id: row.get(0),
                symbol: row.get(1),
                status: row.get(2),
                base_coin: row.get(3),
                quote_coin: row.get(4),
            })
            .collect())
    }

    pub async fn insert_instruments(
        &self,
        instruments: &mut [Instrument],
    ) -> Result<(), CommonError> {
        let existing_instruments = self.get_instruments(false).await?;

        let mut status_dictionary = DbDictionary::query(&self.pool, "instrument_status").await?;
        let mut coin_dictionary = DbDictionary::query(&self.pool, "coin").await?;

        for instrument in instruments {
            if let Some(existing_instrument) = existing_instruments
                .iter()
                .find(|existing_instrument| existing_instrument.symbol == instrument.symbol)
            {
                instrument.id = existing_instrument.id;
                continue;
            }

            let status_code = status_dictionary
                .get_code(&self.pool, &instrument.status)
                .await?;
            let base_coin_code = coin_dictionary
                .get_code(&self.pool, &instrument.base_coin)
                .await?;
            let quote_coin_code = coin_dictionary
                .get_code(&self.pool, &instrument.quote_coin)
                .await?;

            info!("Insert {:?}", instrument);
            let id_row = sqlx::query("INSERT INTO instrument (symbol, status_code, base_coin_code, quote_coin_code) VALUES ($1, $2, $3, $4) RETURNING id")
                .bind(instrument.symbol.clone())
                .bind(status_code)
                .bind(base_coin_code)
                .bind(quote_coin_code)
                .fetch_one(&self.pool).await?;
            let id = id_row.get(0);
            instrument.id = Some(id);
        }

        Ok(())
    }

    async fn serialize_delta(&self, delta: &[OrderData]) -> Result<Vec<u8>, CommonError> {
        let mut delta_vec = Vec::with_capacity(delta.len());
        for data in delta {
            delta_vec.push(OrderDataDelta {
                price_id: self.prices.get_id(data.price, &self.pool).await?,
                size: data.size,
            });
        }
        let res = bincode::serde::encode_to_vec(&delta_vec, bincode::config::standard())?;
        Ok(res)
    }

    fn deserialize_and_handle(data: &[u8], cur_data: &mut Vec<OrderData>, prices_map: &HashMap<i32, Decimal>) -> Result<bool, CommonError> {
        let (delta_vec, _): (Vec<OrderDataDelta>, usize) = bincode::serde::decode_from_slice(data, bincode::config::standard())?;
        let mut deltas = Vec::with_capacity(delta_vec.len());
        for delta in delta_vec {
            if let Some(price) = prices_map.get(&delta.price_id).copied() {
                deltas.push(OrderData { price, size: delta.size });
            } else {
                return Ok(false);
            }
        }
        handle_delta(&deltas, cur_data);
        Ok(true)
    }

    pub async fn get_order_books(
        &self,
        instrument_id: Option<i16>,
        diapason: TimeDiapason,
        ab: QueryAsksOrBids,
    ) -> Result<Vec<OrderBook>, CommonError> {
        let instrument_id = instrument_id.ok_or::<CommonError>("Instrument has no id".into())?;

        info!("Query order books instrument {instrument_id} {diapason:?}");
        let rows = sqlx::query("SELECT id, created, ask_delta, bid_delta FROM order_book WHERE instrument_id = $1 AND created BETWEEN $2 AND $3 ORDER BY created")
            .bind(instrument_id)
            .bind(diapason.from)
            .bind(diapason.to)
            .fetch_all(&self.pool).await?;

        let full_ids = rows
            .iter()
            .filter(|row| {
                let v: Option<&[u8]> = row.get(2);
                v.is_none()
            })
            .map(|row| {
                let id: i64 = row.get(0);
                id
            })
            .collect::<Vec<i64>>();
        let mut full_asks = if ab.is_asks() {
            self.get_order_data(&full_ids, "order_book_ask").await?
        } else {
            HashMap::default()
        };
        let mut full_bids = if ab.is_bids() {
            self.get_order_data(&full_ids, "order_book_bid").await?
        } else {
            HashMap::default()
        };

        let mut prices_map = HashMap::new();
        self.prices.get_prices(&mut prices_map).await;
        let mut cur_asks = Vec::new();
        let mut cur_bids = Vec::new();

        let mut result = Vec::new();
        let mut prev_percentage = 0;
        for (i, row) in rows.iter().enumerate() {
            let id = row.get(0);
            let asks_encoded: Option<Vec<u8>> = row.get(2);
            let bids_encoded: Option<Vec<u8>> = row.get(3);
            let mut order_book = OrderBook {
                id: Some(id),
                timestamp: row.get(1),
                asks: Vec::default(),
                bids: Vec::default(),
            };

            if ab.is_asks() {
                if let Some(asks) = full_asks.remove(&id) {
                    cur_asks = asks.clone();
                    order_book.asks = asks;
                } else if let Some(asks_encoded) = asks_encoded {
                    loop {
                        if Self::deserialize_and_handle(&asks_encoded, &mut cur_asks, &prices_map)? {
                            break;
                        }
                        self.prices.init(&self.pool).await?;
                        self.prices.get_prices(&mut prices_map).await;
                    }
                    order_book.asks = cur_asks.clone();
                }
            }
            if ab.is_bids() {
                if let Some(bids) = full_bids.remove(&id) {
                    cur_bids = bids.clone();
                    order_book.bids = bids;
                } else if let Some(bids_encoded) = bids_encoded {
                    loop {
                        if Self::deserialize_and_handle(&bids_encoded, &mut cur_bids, &prices_map)? {
                            break;
                        }
                        self.prices.init(&self.pool).await?;
                        self.prices.get_prices(&mut prices_map).await;
                    }
                    order_book.bids = cur_bids.clone();
                }
            }

            if !order_book.asks.is_empty() || !order_book.bids.is_empty() {
                result.push(order_book);
            }

            let percentage = i * 100 / rows.len();
            if percentage > prev_percentage {
                debug!("Process {percentage}%");
                prev_percentage = percentage;
            }
        }
        Ok(result)
    }

    async fn get_order_data(
        &self,
        order_book_ids: &[i64],
        table_name: &str,
    ) -> Result<HashMap<i64, Vec<OrderData>>, CommonError> {
        let rows = sqlx::query(
            &format!("SELECT o.order_book_id, p.price, o.size FROM {table_name} o JOIN price p ON o.price_id = p.id WHERE order_book_id IN (SELECT * FROM unnest($1))"))
            .bind(order_book_ids)
            .fetch_all(&self.pool).await?;
        let mut result: HashMap<i64, Vec<OrderData>> = HashMap::with_capacity(order_book_ids.len());
        for row in rows {
            let order_book_id = row.get(0);
            let data = OrderData {
                price: row.get(1),
                size: row.get(2),
            };
            if let Some(vec) = result.get_mut(&order_book_id) {
                vec.push(data);
            } else {
                result.insert(order_book_id, vec![data]);
            }
        }
        Ok(result)
    }

    async fn insert_order_book(
        &self,
        order_book: &OrderBook,
        instrument: &Instrument,
    ) -> Result<i64, CommonError> {
        let instrument_id = instrument
            .id
            .ok_or::<CommonError>(format!("Instrument {} has no id", instrument.symbol).into())?;
        let res = sqlx::query(
            "INSERT INTO order_book (created, instrument_id) VALUES ($1, $2) RETURNING id",
        )
        .bind(order_book.timestamp)
        .bind(instrument_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(res.get(0))
    }

    pub async fn insert_order_book_delta(
        &self,
        instrument: &Instrument,
        timestamp: DateTime<Utc>,
        ask_delta: &[OrderData],
        bid_delta: &[OrderData],
    ) -> Result<(), CommonError> {
        let instrument_id = instrument
            .id
            .ok_or::<CommonError>(format!("Instrument {} has no id", instrument.symbol).into())?;

        let ask_serialized = self.serialize_delta(ask_delta).await?;
        let bid_serialized = self.serialize_delta(bid_delta).await?;

        sqlx::query("INSERT INTO order_book (created, instrument_id, ask_delta, bid_delta) VALUES ($1, $2, $3, $4)")
            .bind(timestamp)
            .bind(instrument_id)
            .bind(ask_serialized)
            .bind(bid_serialized)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn insert_order_book_full(
        &self,
        order_book: &OrderBook,
        instrument: &Instrument,
    ) -> Result<(), CommonError> {
        let id = self.insert_order_book(order_book, instrument).await?;

        let mut asks_ids = Vec::with_capacity(order_book.asks.len());
        let mut asks_sizes = Vec::with_capacity(order_book.asks.len());
        for ask in &order_book.asks {
            let price_id = self.prices.get_id(ask.price, &self.pool).await?;
            asks_ids.push(price_id);
            asks_sizes.push(ask.size);
        }
        let mut bids_ids = Vec::with_capacity(order_book.bids.len());
        let mut bids_sizes = Vec::with_capacity(order_book.bids.len());
        for bid in &order_book.bids {
            let price_id = self.prices.get_id(bid.price, &self.pool).await?;
            bids_ids.push(price_id);
            bids_sizes.push(bid.size);
        }

        sqlx::query("INSERT INTO order_book_ask (order_book_id, price_id, size) SELECT * FROM unnest($1, $2, $3)")
            .bind(vec![id; order_book.asks.len()])
            .bind(asks_ids)
            .bind(asks_sizes)
            .execute(&self.pool).await?;

        sqlx::query("INSERT INTO order_book_bid (order_book_id, price_id, size) SELECT * FROM unnest($1, $2, $3)")
            .bind(vec![id; order_book.bids.len()])
            .bind(bids_ids)
            .bind(bids_sizes)
            .execute(&self.pool).await?;

        Ok(())
    }
}
