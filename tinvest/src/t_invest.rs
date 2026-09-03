use chrono::{DateTime, TimeZone, Utc};
use log::{error, info, warn};
use model::common::{CommonError, EmptyResult};
use model::events::{OrderBookDeltaEvent, OrderBookEvent};
use model::{Instrument, OrderBook, OrderData, Trade};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::ops::Add;
use t_invest_sdk::TInvestSdk;
use t_invest_sdk::api::market_data_response::Payload;
use t_invest_sdk::api::{market_data_request, InstrumentStatus, InstrumentsRequest, MarketDataRequest, OrderBookInstrument, OrderBookType, PingRequest, Quotation, SubscribeOrderBookRequest, SubscribeTradesRequest, SubscriptionAction, TradeInstrument, TradeSourceType, TradeDirection};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Clone)]
pub struct TInvest {
    sdk: TInvestSdk,
}

impl TInvest {
    pub async fn new(token: &str) -> Result<Self, CommonError> {
        let sdk = TInvestSdk::new_production(token).await?;
        info!("SDK initialized");
        Ok(Self { sdk })
    }

    pub async fn list_futures(&self) -> Result<Vec<Instrument>, CommonError> {
        let futures_response = self.sdk
            .instruments()
            .futures(InstrumentsRequest {
                instrument_status: Some(InstrumentStatus::Base as i32),
                instrument_exchange: None,
            }).await?
            .into_inner();
        info!("Got {} futures", futures_response.instruments.len());
        let instruments = futures_response.instruments.into_iter()
            .filter(|future| future.api_trade_available_flag && future.short_enabled_flag)
            .map(|future| Instrument {
                id: None,
                ticker: future.ticker,
                external_id: Some(future.uid),
                name: future.name,
                status: future.trading_status.to_string(),
                base_coin: Some(future.currency),
                quote_coin: None,
            }).collect();
        Ok(instruments)
    }

    pub async fn listen_trades(
        &self,
        instruments_id_uid: Vec<(String, i16)>,
        trade_sender: mpsc::Sender<Trade>,
    ) -> EmptyResult {
        let mut uid_id_map = HashMap::with_capacity(instruments_id_uid.len());
        for (uid, id) in &instruments_id_uid {
            uid_id_map.insert(uid.clone(), id.clone());
        }
        let subs = instruments_id_uid.into_iter()
            .map(|(instrument_id, _)| {
                TradeInstrument {
                    instrument_id,
                    figi: String::new(),
                }
            }).collect::<Vec<_>>();
        let request = MarketDataRequest {
            payload: Some(market_data_request::Payload::SubscribeTradesRequest(
                SubscribeTradesRequest {
                    subscription_action: SubscriptionAction::Subscribe as i32,
                    trade_source: TradeSourceType::TradeSourceAll as i32,
                    with_open_interest: false,
                    instruments: subs,
                }
            ))
        };
        let (tx, rx) = mpsc::unbounded_channel::<MarketDataRequest>();
        tx.send(request)?;

        let response = self.sdk
            .market_data_stream()
            .market_data_stream(UnboundedReceiverStream::new(rx))
            .await?;

        let mut streaming = response.into_inner();
        while let Some(message) = streaming.message().await? {
            if let Some(payload) = message.payload {
                match payload {
                    Payload::SubscribeTradesResponse(response) => {
                        info!("Trade subscribe response: {response:?}");
                    }
                    Payload::Trade(trade) => {
                        if let Some(&instrument_id) = uid_id_map.get(&trade.instrument_uid) {
                            let trade = Trade {
                                id: None,
                                instrument_id,
                                created: convert_timestamp(trade.time.map(|t| (t.seconds, t.nanos))),
                                quantity: Decimal::from(trade.quantity),
                                price: trade.price.map(|price| convert_quotation(&price)),
                                direction: decode_trade_direction(trade.direction),
                            };
                            if let Err(err) = trade_sender.send(trade).await {
                                error!("Error sending trade: {}", err);
                            }
                        }
                    }
                    Payload::Ping(ping) => {
                        let request = MarketDataRequest {
                            payload: Some(market_data_request::Payload::Ping(
                                PingRequest {
                                    time: ping.time,
                                }
                            )),
                        };
                        tx.send(request)?;
                    }
                    _ => {
                        warn!("Ignore message: {:?}", payload);
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn listen(
        &self,
        instruments_id_uid: Vec<(String, i16)>,
        order_book_sender: mpsc::Sender<OrderBookEvent>,
        order_book_delta_sender: mpsc::Sender<OrderBookDeltaEvent>,
    ) -> EmptyResult {
        let mut uid_id_map = HashMap::with_capacity(instruments_id_uid.len());
        for (uid, id) in &instruments_id_uid {
            uid_id_map.insert(uid.clone(), id.clone());
        }
        let subs = instruments_id_uid.into_iter()
            .map(|(instrument_id, _)| {
                OrderBookInstrument {
                    instrument_id,
                    depth: 50,
                    order_book_type: OrderBookType::OrderbookTypeAll as i32,
                    figi: "".to_string(),
                }
            })
            .collect::<Vec<_>>();
        let request = MarketDataRequest {
            payload: Some(market_data_request::Payload::SubscribeOrderBookRequest(
                SubscribeOrderBookRequest {
                    subscription_action: SubscriptionAction::Subscribe as i32,
                    instruments: subs,
                },
            )),
        };
        let (tx, rx) = mpsc::unbounded_channel::<MarketDataRequest>();
        tx.send(request)?;

        let response = self.sdk
            .market_data_stream()
            .market_data_stream(UnboundedReceiverStream::new(rx))
            .await?;

        let mut context = ListenerContext {
            uid_id_map,
            order_book_sender,
            order_book_delta_sender,
            order_books: HashMap::new(),
        };
        let mut streaming = response.into_inner();
        while let Some(message) = streaming.message().await? {
            if let Some(payload) = message.payload {
                match payload {
                    Payload::SubscribeOrderBookResponse(response) => {
                        info!("Order book subscribe response: {response:?}");
                    }
                    Payload::Orderbook(order_book) => {
                        if let Err(err) = handle_order_book(&mut context, order_book).await {
                            error!("Error handling orderbook: {err}");
                        }
                    }
                    Payload::Ping(ping) => {
                        let request = MarketDataRequest {
                            payload: Some(market_data_request::Payload::Ping(
                                PingRequest {
                                    time: ping.time,
                                }
                            )),
                        };
                        tx.send(request)?;
                    }
                    _ => {
                        warn!("Ignore message: {:?}", payload);
                    }
                }
            }
        }
        Ok(())
    }
}

struct ListenerContext {
    uid_id_map: HashMap<String, i16>,
    order_books: HashMap<String, OrderBook>,
    order_book_sender: mpsc::Sender<OrderBookEvent>,
    order_book_delta_sender: mpsc::Sender<OrderBookDeltaEvent>,
}

async fn handle_order_book(context: &mut ListenerContext, api_order_book: t_invest_sdk::api::OrderBook) -> EmptyResult {
    if !api_order_book.is_consistent {
        warn!("Order book is not consistent: {api_order_book:?}");
    }
    let new_order_book = convert_order_book(&api_order_book);
    let mut prev_order_book = OrderBook {
        id: None,
        timestamp: new_order_book.timestamp,
        asks: Vec::new(),
        bids: Vec::new(),
    };
    if let Some(order_books) = context.order_books.get(&api_order_book.instrument_uid) {
        prev_order_book = order_books.clone();
    }
    let instrument_id = context.uid_id_map.get(&api_order_book.instrument_uid).copied().unwrap_or_default();
    if prev_order_book.asks.is_empty() || prev_order_book.bids.is_empty() {
        context.order_book_sender.send(OrderBookEvent {
            instrument_id,
            order_book: new_order_book.clone(),
        }).await?;
    } else {
        let ask_delta = calculate_delta(&prev_order_book.asks, &new_order_book.asks);
        let bid_delta = calculate_delta(&prev_order_book.bids, &new_order_book.bids);
        context.order_book_delta_sender.send(OrderBookDeltaEvent {
            instrument_id,
            timestamp: new_order_book.timestamp,
            asks: ask_delta,
            bids: bid_delta,
        }).await?;
    }
    context.order_books.insert(api_order_book.instrument_uid, new_order_book);
    Ok(())
}

fn calculate_delta(prev_orders: &[OrderData], new_orders: &[OrderData]) -> Vec<OrderData> {
    let mut delta = new_orders.iter()
        .filter(|new| {
            if let Some(prev) = prev_orders.iter().find(|prev| prev.price == new.price) {
                prev.size != new.size
            } else {
                true
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    delta.extend(prev_orders.iter()
        .filter_map(|prev| {
            if new_orders.iter().any(|new| new.price == prev.price) {
                None
            } else {
                Some(OrderData {
                    price: prev.price,
                    size: Decimal::ZERO,
                })
            }
        }));
    delta
}

fn convert_order_book(order_book: &t_invest_sdk::api::OrderBook) -> OrderBook {
    OrderBook {
        id: None,
        timestamp: convert_timestamp(order_book.time.map(|t|  (t.seconds, t.nanos))),
        asks: convert_orders(&order_book.asks),
        bids: convert_orders(&order_book.bids),
    }
}

fn convert_orders(order: &[t_invest_sdk::api::Order]) -> Vec<OrderData> {
    order.iter()
        .filter_map(|order| {
            order.price.as_ref().map(|price| {
                OrderData {
                    price: convert_quotation(price),
                    size: Decimal::from(order.quantity),
                }
            })
        }).collect::<Vec<_>>()
}

fn convert_timestamp(time: Option<(i64, i32)>) -> DateTime<Utc> {
    time.and_then(|(s, ns)| Utc.timestamp_opt(s, ns as u32).earliest())
        .unwrap_or(Utc::now())
}

fn convert_quotation(quotation: &Quotation) -> Decimal {
    Decimal::from(quotation.units)
        .add(Decimal::from_i128_with_scale(quotation.nano as i128, 9))
}

fn decode_trade_direction(dir: i32) -> Option<char> {
    if TradeDirection::Buy as i32 == dir {
        Some('b')
    } else if TradeDirection::Sell as i32 == dir {
        Some('s')
    } else {
        None
    }
}