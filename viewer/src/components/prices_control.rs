use std::collections::HashSet;
use leptos::{component, view, IntoView};
use leptos::prelude::*;
use thaw::{Label, Flex, CheckboxGroup, Checkbox};
use crate::client_model::{InstrumentOption, PricesParams};

#[component]
pub fn PricesControl(instruments: Signal<Vec<InstrumentOption>>, params: RwSignal<PricesParams>, is_req_allowed: RwSignal<bool>) -> impl IntoView {
    let instruments_values = RwSignal::new(HashSet::<String>::new());

    Effect::new(move || {
        let new_params = PricesParams {
            instrument_ids_asks: instruments_values.get().iter()
                .filter(|val| val.ends_with("true"))
                .map(|val| val.split(".").next().unwrap().parse().unwrap_or_default())
                .collect(),
            instrument_ids_bids: instruments_values.get().iter()
                .filter(|val| val.ends_with("false"))
                .map(|val| val.split(".").next().unwrap().parse().unwrap_or_default())
                .collect(),
        };
        is_req_allowed.set(!new_params.instrument_ids_asks.is_empty() || !new_params.instrument_ids_bids.is_empty());
        params.set(new_params);
    });

    let is_req_disabled = Signal::derive(move || instruments_values.get().is_empty());

    view! {
        <div style="display: flex; width: 100%">
            <Flex vertical=true>
                <Label>"Инструменты"</Label>
                <CheckboxGroup value=instruments_values>
                    <For
                        each = move || instruments.get()
                        key = |instrument| instrument.id.clone()
                        let (instrument)
                    >
                        <Checkbox label=instrument.label value=instrument.id
                        />
                    </For>
                </CheckboxGroup>
            </Flex>
        </div>
    }
}
