use chrono::{DateTime, Datelike, TimeDelta, Utc};
use db::{Db, QueryAsksOrBids};
use log::{debug, info, LevelFilter};
use model::common::{CommonError, EmptyResult, TimeDiapason};
use model::{Instrument, OrderBook, order_book_cache};
use rust_decimal::Decimal;
use signal::{Signal, TradeSignal};
use simplelog::SimpleLogger;

#[derive(Copy, Clone)]
struct TestDeal {
    signal: TradeSignal,
    open_time1: DateTime<Utc>,
    open_time2: DateTime<Utc>,
    close_time: DateTime<Utc>,
    open_price1: Decimal,
    open_price2: Decimal,
    close_time1: DateTime<Utc>,
    close_time2: DateTime<Utc>,
    close_price1: Option<Decimal>,
    close_price2: Option<Decimal>,
}

impl TestDeal {
    fn open(signal: TradeSignal, ob1: &OrderBook, ob2: &OrderBook) -> Option<Self> {
        match signal {
            TradeSignal::Sell1Buy2(hold) => {
                Some(TestDeal {
                    signal,
                    open_time1: ob1.timestamp,
                    open_time2: ob2.timestamp,
                    close_time1: ob1.timestamp,
                    close_time2: ob2.timestamp,
                    close_time: ob1.timestamp.max(ob2.timestamp) + TimeDelta::milliseconds(hold as i64),
                    open_price1: get_best_bid(ob1),
                    open_price2: get_best_ask(ob2),
                    close_price1: None,
                    close_price2: None,
                })
            }
            TradeSignal::Buy1Sell2(hold) => {
                Some(TestDeal {
                    signal,
                    open_time1: ob1.timestamp,
                    open_time2: ob2.timestamp,
                    close_time1: ob1.timestamp,
                    close_time2: ob2.timestamp,
                    close_time: ob1.timestamp.max(ob2.timestamp) + TimeDelta::milliseconds(hold as i64),
                    open_price1: get_best_ask(ob1),
                    open_price2: get_best_bid(ob2),
                    close_price1: None,
                    close_price2: None,
                })
            }
            _ => {
                None
            }
        }
    }

    fn close1(&mut self, ob: &OrderBook) {
        if self.close_price1.is_some() {
            return;
        }

        self.close_time1 = ob.timestamp;
        match self.signal {
            TradeSignal::Sell1Buy2(_) => {
                self.close_price1 = Some(get_best_ask(ob));
            }
            TradeSignal::Buy1Sell2(_) => {
                self.close_price1 = Some(get_best_bid(ob));
            }
            _ => {}
        }
    }

    fn close2(&mut self, ob: &OrderBook) {
        if self.close_price2.is_some() {
            return;
        }

        self.close_time2 = ob.timestamp;
        match self.signal {
            TradeSignal::Sell1Buy2(_) => {
                self.close_price2 = Some(get_best_bid(ob));
            }
            TradeSignal::Buy1Sell2(_) => {
                self.close_price2 = Some(get_best_ask(ob));
            }
            _ => {}
        }
    }

    fn close(&self) -> (Decimal, Decimal) {
        if let Some(close_price1) = self.close_price1 && let Some(close_price2) = self.close_price2 {
            match self.signal {
                TradeSignal::Sell1Buy2(_) => {
                    debug!(
                        "Sell 1 @ {} at {} buy 2 @ {} at {} -> buy 1 @ {} at {} sell 2 @ {} at {}",
                        self.open_price1, self.open_time1, self.open_price2, self.open_time2,
                        close_price1, self.close_time1, close_price2, self.close_time2,
                    );
                    (self.open_price1 + close_price2, self.open_price2 + close_price1)
                },
                TradeSignal::Buy1Sell2(_) => {
                    debug!(
                        "Buy 1 @ {} at {} sell 2 @ {} at {} -> sell 1 @ {} at {} buy 2 @ {} at {}",
                        self.open_price1, self.open_time1, self.open_price2, self.open_time2,
                        close_price1, self.close_time1, close_price2, self.close_time2,
                    );
                    (self.open_price2 + close_price1, self.open_price1 + close_price2)
                },
                _ => unreachable!(),
            }
        } else {
            (Decimal::ZERO, Decimal::ZERO)
        }
    }
}

struct OrderBookEvent {
    is_first_leg: bool,
    order_book: OrderBook,
}

const SLIPPERING_MS: u16 = 0;

#[tokio::main]
async fn main() -> EmptyResult {
    dotenv::dotenv().ok();
    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let db = db::Db::new(&db_url).await?;

    let tickers = ["GLU6", "GLZ6"];

    let mut signal = Signal::new(tickers[0], tickers[1]);
    let all_instruments = db.get_instruments(false).await?;
    if let (Some(inst1_id), Some(inst2_id)) = (
        find_instrument_id(&all_instruments, tickers[0]),
        find_instrument_id(&all_instruments, tickers[1]),
    ) {
        let mut diapason = TimeDiapason::new(
            DateTime::parse_from_rfc3339("2026-09-03T05:00:00Z")?.to_utc(),
            DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")?.to_utc(),
        );

        let (mut total_income, mut total_outcome, mut total_commission) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
        let mut active_deal = Option::<TestDeal>::None;

        while diapason.to + TimeDelta::days(1) < Utc::now() {
            if !matches!(diapason.from.weekday().number_from_monday(), 6 | 7) {
                info!("DAY {}", diapason.from.date_naive());
                let (mut daily_income, mut daily_outcome, mut daily_commission) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
                let mut daily_deals = 0;

                let (order_books1, order_books2) = get_order_books(&tickers, &[inst1_id, inst2_id], diapason, Some(&db)).await?;
                info!("{} {} order books, {} {} order books", order_books1.len(), tickers[0], order_books2.len(), tickers[1]);

                let events = merge_events(&order_books1, &order_books2);

                let (mut last_ob1, mut last_ob2) = (None, None);
                for (i, event) in events.iter().enumerate() {
                    signal.calibrate();
                    let trade_signal = signal.process(
                        if event.is_first_leg {tickers[0]} else {tickers[1]},
                        &event.order_book,
                    );

                    if event.is_first_leg {
                        last_ob1 = Some(event.order_book.clone());
                    } else {
                        last_ob2 = Some(event.order_book.clone());
                    }

                    if let Some(ob1) = &last_ob1 && let Some(ob2) = &last_ob2 {
                        if let Some(deal) = active_deal.as_mut() {
                            if ob1.timestamp > deal.close_time {
                                if let Some((hob1, _)) = on_horizon(&events, i, SLIPPERING_MS) {
                                    deal.close1(hob1);
                                } else {
                                    deal.close1(ob1);
                                }
                            }
                            if ob2.timestamp > deal.close_time {
                                if let Some((_, hob2)) = on_horizon(&events, i, SLIPPERING_MS) {
                                    deal.close2(hob2);
                                } else {
                                    deal.close2(ob2);
                                }
                            }

                            if deal.close_price1.is_some() && deal.close_price2.is_some() {
                                let (revenue, cost) = deal.close();
                                total_income += revenue; daily_income += revenue;
                                total_outcome += cost; daily_outcome += cost;
                                total_commission += Decimal::from(5); daily_commission += Decimal::from(5);
                                daily_deals += 1;
                                active_deal = None;
                            }
                        } else if matches!(trade_signal, TradeSignal::Buy1Sell2(_) | TradeSignal::Sell1Buy2(_)) {
                            if let Some((hob1, hob2)) = on_horizon(&events, i, SLIPPERING_MS) {
                                active_deal = TestDeal::open(trade_signal, hob1, hob2);
                            } else {
                                active_deal = TestDeal::open(trade_signal, ob1, ob2);
                            }
                        }
                    }
                }

                info!("Daily profit {} on {} deals with commission {}\n", daily_income - daily_outcome - daily_commission, daily_deals, daily_commission);
            }

            diapason.from += TimeDelta::days(1);
            diapason.to += TimeDelta::days(1);
        }
        info!("Total profit {}", total_income - total_outcome - total_commission);
    }
    Ok(())
}

fn on_horizon(events: &[OrderBookEvent], i: usize, horizon: u16) -> Option<(&OrderBook, &OrderBook)> {
    if horizon == 0 {
        return None;
    }

    let mut index = i;
    let target_time = events[i].order_book.timestamp + TimeDelta::milliseconds(horizon as i64);

    let (mut i1, mut i2) = (None, None);
    while index < events.len() {
        if events[index].order_book.timestamp >= target_time {
            if events[index].is_first_leg {
                i1 = Some(index);
            } else {
                i2 = Some(index);
            }
        }

        if let Some(i1) = i1 && let Some(i2) = i2 {
            return Some((&events[i1].order_book, &events[i2].order_book));
        }

        index += 1;
    }

    None
}

fn get_best_ask(order_book: &OrderBook) -> Decimal {
    order_book.asks.iter().map(|data| data.price).min().unwrap()
}

fn get_best_bid(order_book: &OrderBook) -> Decimal {
    order_book.bids.iter().map(|data| data.price).max().unwrap()
}

fn merge_events(values1: &[OrderBook], values2: &[OrderBook]) -> Vec<OrderBookEvent> {
    let mut events = Vec::with_capacity(values1.len() + values2.len());
    let (mut v_index1, mut v_index2, mut t_index1, mut t_index2) = (0, 0, 0, 0);
    while v_index1 < values1.len() || v_index2 < values2.len() {
        let mut cur_event = None;
        let mut cur_event_time = None;

        if let Some(v) = values1.get(v_index1) {
            let use_it = if let Some(event_time) = cur_event_time {
                v.timestamp < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(v.timestamp);
                cur_event = Some(OrderBookEvent { is_first_leg: true, order_book: v.clone() });
            }
        }

        if let Some(v) = values2.get(v_index2) {
            let use_it = if let Some(event_time) = cur_event_time {
                v.timestamp < event_time
            } else { true };
            if use_it {
                cur_event_time = Some(v.timestamp);
                cur_event = Some(OrderBookEvent { is_first_leg: false, order_book: v.clone() });
            }
        }

        if let Some(event) = cur_event {
            if event.is_first_leg {
                v_index1 += 1;
            } else {
                v_index2 += 1;
            }
            events.push(event);
        } else {
            break;
        }
    }
    events
}


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
