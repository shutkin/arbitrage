use std::str::FromStr;
use chrono::{DateTime, FixedOffset};
use leptos::server_fn::serde::{Deserialize, Serialize};

pub const NAN_VALUE: f64 = -1e12;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClientInstrument {
    pub id: i16,
    pub symbol: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum PriceCalculation {
    #[default]
    FilteredBest,
    WeightedAverage,
}

impl FromStr for PriceCalculation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s { 
            "0" => Ok(PriceCalculation::FilteredBest),
            "1" => Ok(PriceCalculation::WeightedAverage),
            _ => Err(format!("Unknown price calculation: {s}")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MainParams {
    pub interval_minutes: i16,
    pub date_time: DateTime<FixedOffset>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PricesParams {
    pub instrument_ids_asks: Vec<i16>,
    pub instrument_ids_bids: Vec<i16>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DeltaParams {
    pub instrument1: i16,
    pub instrument2: i16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TimedValues {
    pub timestamp: DateTime<FixedOffset>,
    pub values: Vec<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PricesResponse {
    pub names: Vec<String>,
    pub data: Vec<TimedValues>,
}

#[derive(Clone, PartialEq)]
pub struct InstrumentOption {
    pub id: String,
    pub label: String,
}

pub fn process_chart_data(data: &[TimedValues]) -> Vec<TimedValues> {
    data.iter().map(|item| {
        TimedValues {
            timestamp: item.timestamp,
            values: item.values.iter().map(|v| if *v > NAN_VALUE {*v} else {f64::NAN}).collect(),
        }
    }).collect()
}
