use chrono::Utc;
use db::Db;
use log::{debug, error, info, warn};
use model::common::EmptyResult;
use model::events::{OrderBookDeltaEvent, OrderBookEvent};
use model::utils::handle_delta;
use model::{Instrument, OrderBook, Trade};
use simplelog::{LevelFilter, SimpleLogger};
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tinvest::TInvest;
use tokio::sync::mpsc;

const FULL_ORDER_BOOK_QUANT_MINUTES: i64 = 10;

#[derive(Clone)]
struct ListenerHandler {
    db: Db,
    instruments: HashMap<i16, Instrument>,
    order_books: Arc<Mutex<HashMap<i16, OrderBook>>>,
    ticker: Arc<AtomicI64>,
}

impl ListenerHandler {
    async fn on_trade(&self, trade: &Trade) {
        self.ticker.store(Utc::now().timestamp(), Relaxed);
        if let Err(err) = self.db.insert_trade(trade).await {
            error!("Failed to insert trade: {err}");
        }
    }

    async fn on_order_book(&self, event: OrderBookEvent) {
        self.ticker.store(Utc::now().timestamp(), Relaxed);
        if let Some(instrument) = self.instruments.get(&event.instrument_id) {
            info!("Save snapshot order book {:?}", event.order_book);
            if let Err(err) = self.db.insert_order_book_full(&event.order_book, instrument).await {
                error!("Failed to save order book: {}", err);
            }
            self.order_books.lock().unwrap().insert(event.instrument_id, event.order_book);
        }
    }

    async fn on_order_book_delta(&self, event: OrderBookDeltaEvent) {
        self.ticker.store(Utc::now().timestamp(), Relaxed);
        if let Some(instrument) = self.instruments.get(&event.instrument_id) {
            let mut updated_order_book = None;
            if let Some(order_book) = self.order_books.lock().unwrap().get_mut(&event.instrument_id) {
                let prev_timestamp_quant = order_book.timestamp.timestamp_micros() / (FULL_ORDER_BOOK_QUANT_MINUTES * 60 * 1000 * 1000);
                order_book.timestamp = event.timestamp;
                handle_delta(&event.asks, &mut order_book.asks);
                handle_delta(&event.bids, &mut order_book.bids);
                let cur_timestamp_quant = order_book.timestamp.timestamp_micros() / (FULL_ORDER_BOOK_QUANT_MINUTES * 60 * 1000 * 1000);
                if cur_timestamp_quant > prev_timestamp_quant {
                    updated_order_book = Some(order_book.clone());
                }
            }
            if let Some(order_book) = updated_order_book {
                info!("Save full order book {order_book:?}");
                if let Err(err) = self.db.insert_order_book_full(&order_book, instrument).await {
                    error!("Failed to save order book: {}", err);
                }
            } else {
                debug!("Save order book delta asks {:?}, bids {:?}", event.asks, event.bids);
                if let Err(err) = self.db.insert_order_book_delta(instrument, event.timestamp, &event.asks, &event.bids).await {
                    error!("Failed to save order book delta: {}", err);
                }
            }
        } else {
            warn!("Skipping order book for unknown instrument {}", event.instrument_id);
        }
    }
}

#[tokio::main]
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();

    let token = std::env::var("T_INVEST_TOKEN").expect("T_INVEST_TOKEN is not set");
    let t_invest = TInvest::new(&token).await?;

    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = Db::new(&db_url).await?;

    let mut instruments = t_invest.list_futures().await?;
    db.insert_instruments(&mut instruments).await?;
    let tickers = ["GLU6", "GLZ6", "GLH7", "GLM7"];
    let mut instruments_map = HashMap::new();
    instruments.iter().for_each(|instrument| {
        if tickers.contains(&instrument.ticker.as_str()) && let Some(id) = instrument.id {
            instruments_map.insert(id, instrument.clone());
        }
    });

    let db_clone = db.clone();
    let token_clone = token.clone();
    let instruments_map_clone = instruments_map.clone();
    tokio::spawn(async move {
        let ticker = Arc::new(AtomicI64::new(0));
        loop {
            let now = Utc::now().timestamp();
            if ticker.load(Relaxed) < now - 5 * 60 {
                start_listener(db_clone.clone(), token_clone.clone(), instruments_map_clone.clone(), ticker.clone());
                ticker.store(Utc::now().timestamp(), Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let ticker = Arc::new(AtomicI64::new(0));
    loop {
        let now = Utc::now().timestamp();
        if ticker.load(Relaxed) < now - 5 * 60 {
            start_trade_listener(db.clone(), token.clone(), instruments_map.clone(), ticker.clone());
            ticker.store(Utc::now().timestamp(), Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn start_listener(db: Db, token: String, instruments: HashMap<i16, Instrument>, ticker: Arc<AtomicI64>) {
    let (order_book_sender,
        mut order_book_receiver) = mpsc::channel::<OrderBookEvent>(64);
    let (order_book_delta_sender,
        mut order_book_delta_receiver) = mpsc::channel::<OrderBookDeltaEvent>(64);
    let listener_handler = ListenerHandler {
        db: db.clone(),
        instruments: instruments.clone(),
        order_books: Arc::new(Mutex::new(HashMap::new())),
        ticker: ticker.clone(),
    };
    let listener_handler_clone = listener_handler.clone();
    tokio::spawn(async move {
        while let Some(event) = order_book_receiver.recv().await {
            listener_handler_clone.on_order_book(event).await;
        }
    });
    tokio::spawn(async move {
        while let Some(event) = order_book_delta_receiver.recv().await {
            listener_handler.on_order_book_delta(event).await;
        }
    });

    let instruments_ids = instruments.values()
        .filter_map(|instrument| {
            if let Some(id) = instrument.id && let Some(external_id) = &instrument.external_id {
                Some((external_id.clone(), id))
            } else {
                None
            }
        }).collect::<Vec<_>>();
    tokio::spawn(async move {
        match TInvest::new(&token).await {
            Ok(t_invest) => {
                info!("Start listen to order books: {instruments_ids:?}");
                if let Err(err) = t_invest.listen(
                    instruments_ids,
                    order_book_sender,
                    order_book_delta_sender,
                ).await {
                    error!("Listener error: {}", err);
                }
            },
            Err(err) => {
                error!("TInvest initialization error: {}", err);
            }
        }
    });
}

fn start_trade_listener(db: Db, token: String, instruments: HashMap<i16, Instrument>, ticker: Arc<AtomicI64>) {
    let instruments_ids = instruments.values()
        .filter_map(|instrument| {
            if let Some(id) = instrument.id && let Some(external_id) = &instrument.external_id {
                Some((external_id.clone(), id))
            } else {
                None
            }
        }).collect::<Vec<_>>();

    let (trade_sender, mut trade_receiver) = mpsc::channel::<Trade>(64);
    let listener_handler = ListenerHandler {
        db: db.clone(),
        instruments: instruments.clone(),
        order_books: Arc::new(Mutex::new(HashMap::new())),
        ticker: ticker.clone(),
    };
    tokio::spawn(async move {
        while let Some(trade) = trade_receiver.recv().await {
            listener_handler.on_trade(&trade).await;
        }
    });
    tokio::spawn(async move {
        match TInvest::new(&token).await {
            Ok(t_invest) => {
                info!("Start listen to trades: {instruments_ids:?}");
                if let Err(err) = t_invest.listen_trades(
                    instruments_ids,
                    trade_sender,
                ).await {
                    error!("Listener error: {}", err);
                }
            },
            Err(err) => {
                error!("TInvest initialization error: {}", err);
            }
        }
    });
}
