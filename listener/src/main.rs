use bybit::BybitListener;
use chrono::Utc;
use db::Db;
use log::{debug, error, info, warn};
use model::events::{OrderBookDeltaEvent, OrderBookEvent};
use model::utils::handle_delta;
use model::{Instrument, OrderBook};
use simplelog::{LevelFilter, SimpleLogger};
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;
use tokio::sync::mpsc;
use model::common::EmptyResult;

const FULL_ORDER_BOOK_QUANT_MINUTES: i64 = 10;

#[derive(Clone)]
struct ListenerHandler {
    db: Db,
    instruments: HashMap<i16, Instrument>,
    order_books: Arc<Mutex<HashMap<i16, OrderBook>>>,
    ticker: Arc<AtomicI64>,
}

impl ListenerHandler {
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
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = Db::new(&db_url).await?;

    let mut instruments = bybit::rest_api::get_instrument_info().await?;
    db.insert_instruments(&mut instruments).await?;
    let symbols = ["BTCPERP", "BTCUSDT-26DEC25", "BTCUSDT-27MAR26", "BTCUSDT-25SEP26",
        "ETHUSDT-26DEC25", "ETHUSDT-27MAR26", "ETHUSDT-26JUN26"].iter().map(ToString::to_string).collect::<Vec<_>>();
    let mut instruments_map = HashMap::new();
    instruments.iter().for_each(|instrument| {
        if symbols.contains(&instrument.symbol) && let Some(id) = instrument.id {
            instruments_map.insert(id, instrument.clone());
        }
    });

    let ticker = Arc::new(AtomicI64::new(0));
    loop {
        let now = Utc::now().timestamp();
        if ticker.load(Relaxed) < now - 5 * 60 {
            start_listener(db.clone(), instruments_map.clone(), ticker.clone());
            ticker.store(Utc::now().timestamp(), Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn start_listener(db: Db, instruments: HashMap<i16, Instrument>, ticker: Arc<AtomicI64>) {
    let (order_book_sender,
        mut order_book_receiver) = mpsc::channel::<OrderBookEvent>(64);
    let (order_book_delta_sender,
        mut order_book_delta_receiver) = mpsc::channel::<OrderBookDeltaEvent>(64);
    let listener_handler = ListenerHandler {
        db: db.clone(),
        instruments: instruments.clone(),
        order_books: Arc::new(Mutex::new(HashMap::new())),
        ticker
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

    let listener = BybitListener::new(
        &instruments.values().cloned().collect::<Vec<_>>(), order_book_sender, order_book_delta_sender);
    tokio::spawn(async move {
        if let Err(err) = listener.start_listen().await {
            error!("Listener error: {}", err);
        }
    });
}
