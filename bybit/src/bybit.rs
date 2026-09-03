pub mod rest_api;

use std::str::FromStr;
use chrono::{DateTime, Utc};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{StreamExt, SinkExt};
use log::{debug, error, info};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use model::{Instrument, OrderBook, OrderData};
use model::common::{CommonError, EmptyResult};
use model::events::{OrderBookDeltaEvent, OrderBookEvent};

const LINEAR_WSS_URL: &str = "wss://stream.bybit.com/v5/public/linear";

#[derive(Serialize)]
struct BybitOperation {
    op: String,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct BybitOrderBookMessage {
    /// Topic name
    topic: String,
    /// Data type: snapshot, delta
    #[serde(rename = "type")]
    item_type: String,
    /// The timestamp (ms) that the system generates the data
    ts: i64,
    /// The timestamp from the matching engine when this orderbook data is produced
    cts: i64,
    data: Option<BybitOrderBookData>,
}

#[derive(Deserialize)]
struct BybitOrderBookData {
    /// Symbol name
    s: String,
    /// Bids: [Bid price, Bid size]
    b: Vec<Vec<String>>,
    /// Asks: [Ask price, Ask size]
    a: Vec<Vec<String>>,
}

fn convert_order_book_item(item: &Vec<String>) -> OrderData {
    OrderData {
        price: Decimal::from_str(&item[0]).unwrap(),
        size: Decimal::from_str(&item[1]).unwrap(),
    }
}

fn convert_bybit_delta(data: &[Vec<String>]) -> Vec<OrderData> {
    data.iter().map(convert_order_book_item).collect()
}

pub struct BybitListener {
    instruments: Vec<Instrument>,
    order_book_sender: Sender<OrderBookEvent>,
    order_book_delta_sender: Sender<OrderBookDeltaEvent>,
}

impl BybitListener {
    pub fn new(instruments: &[Instrument],
               order_book_sender: Sender<OrderBookEvent>,
               order_book_delta_sender: Sender<OrderBookDeltaEvent>) -> Self {
        Self {
            instruments: instruments.to_vec(),
            order_book_sender,
            order_book_delta_sender,
        }
    }

    async fn handle_message(&self, msg_text: &str) -> EmptyResult {
        let msg = serde_json::from_str::<BybitOrderBookMessage>(msg_text)?;
        if let Some(data) = msg.data {
            let instrument = self.instruments.iter()
                .find(|instrument| instrument.ticker == data.s)
                .ok_or::<CommonError>(format!("Unknown instrument {}", data.s).into())?;
            let instrument_id = instrument.id.ok_or::<CommonError>(format!("Instrument {instrument:?} has no id").into())?;
            let timestamp = DateTime::from_timestamp_millis(msg.ts).unwrap_or(Utc::now());

            match msg.item_type.as_str() {
                "snapshot" => {
                    let order_book = OrderBook {
                        id: None,
                        timestamp,
                        asks: data.a.iter().map(convert_order_book_item).collect(),
                        bids: data.b.iter().map(convert_order_book_item).collect(),
                    };
                    self.order_book_sender.send(OrderBookEvent {
                        instrument_id,
                        order_book,
                    }).await?;
                }
                "delta" => {
                    let asks = convert_bybit_delta(&data.a);
                    let bids = convert_bybit_delta(&data.b);
                    self.order_book_delta_sender.send(OrderBookDeltaEvent {
                        instrument_id,
                        timestamp,
                        asks,
                        bids,
                    }).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub async fn start_listen(&self) -> EmptyResult {
        let (mut ws_stream, _) = connect_async(LINEAR_WSS_URL).await?;

        info!("Start listening on instruments: {:?}", self.instruments);

        let operation = BybitOperation {
            op: "subscribe".into(),
            args: self.instruments.iter().map(|instrument| format!("orderbook.50.{}", instrument.ticker)).collect(),
        };
        let json = serde_json::to_string(&operation)?;

        // Send a message
        ws_stream.send(Message::Text(json.into())).await?;

        // Receive messages
        while let Some(msg) = ws_stream.next().await {
            match msg? {
                Message::Text(text) => {
                    if let Err(err) = self.handle_message(&text).await {
                        info!("{text}");
                        error!("Error handling message: {:?}", err);
                    }
                }
                Message::Binary(bin) => println!("Received binary: {:?}", bin),
                Message::Ping(ping) => println!("Received ping: {:?}", ping),
                Message::Pong(pong) => println!("Received pong: {:?}", pong),
                Message::Close(close) => {
                    debug!("Received close: {:?}", close);
                    break;
                }
                Message::Frame(_) => {} // Raw frame, usually handled by the library
            }
        }

        Ok(())
    }
}
