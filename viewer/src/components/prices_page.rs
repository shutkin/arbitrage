use crate::app::get_prices;
use crate::client_model::{process_chart_data, ClientInstrument, InstrumentOption, MainParams, PricesParams, TimedValues};
use crate::components::main_control::MainControl;
use crate::components::prices_chart::PricesChart;
use crate::components::prices_control::PricesControl;
use leptos::leptos_dom::logging::console_error;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use thaw::{Flex, FlexGap};

#[component]
pub fn PricesPage(instruments: Signal<Vec<ClientInstrument>>, is_show: Signal<bool>) -> impl IntoView {
    let instruments_options = Signal::derive(move || {
        let mut list = Vec::new();
        for inst in &instruments.get() {
            list.push(InstrumentOption {
                id: format!("{}.true", inst.id),
                label: format!("{} ask", inst.symbol),
            });
            list.push(InstrumentOption {
                id: format!("{}.false", inst.id),
                label: format!("{} bid", inst.symbol),
            });
        }
        list
    });
    let main_params = RwSignal::new(MainParams::default());
    let params = RwSignal::new(PricesParams::default());
    let is_req_allowed = RwSignal::new(false);

    let action = Action::new(|params: &(MainParams, PricesParams)| {
        let params = params.clone();
        async move {
            get_prices(params).await.unwrap_or_else(|err| {
                console_error(&err.to_string());
                Default::default()
            })
        }
    });

    Effect::new(move |_| {
        let params = params.get();
        if !params.instrument_ids_bids.is_empty() || !params.instrument_ids_asks.is_empty() {
            action.dispatch((main_params.get(), params));
        }
    });
    let prices = Signal::derive(move || action.value().get().unwrap_or_default());

    view! {
        <Show when=move || is_show.get()>
            <Flex vertical=true gap=FlexGap::Large>
                <Flex>
                    <MainControl
                        params=main_params
                        is_loading=Signal::derive(move || action.pending().get())
                        is_req_disabled=Signal::derive(move || !is_req_allowed.get())
                          />
                    <PricesControl
                        instruments=instruments_options
                        params
                        is_req_allowed
                          />
                </Flex>
                <Show when=move || !prices.get().data.is_empty()>
                    <PricesChart
                        data=Signal::derive(move || process_chart_data(&prices.get().data))
                        names=Signal::derive(move || prices.get().names)
                        instruments=instruments_options />
                </Show>
            </Flex>
        </Show>
    }
}
