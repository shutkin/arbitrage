use crate::client_model::{ClientInstrument, DeltaParams};
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use thaw::{Flex, FlexAlign, Label, Select};

#[component]
pub fn DeltaControl(
    instruments: Signal<Vec<ClientInstrument>>,
    params: RwSignal<DeltaParams>,
    is_valid: RwSignal<bool>,
) -> impl IntoView {
    let instrument1 = RwSignal::new(String::default());
    let instrument2 = RwSignal::new(String::default());

    Effect::new(move || {
        let instrument1: i16 = instrument1.get().parse().unwrap_or_default();
        let instrument2: i16 = instrument2.get().parse().unwrap_or_default();
        if instrument1 != 0 && instrument2 != 0 && instrument1 != instrument2 {
            is_valid.set(true);
            params.set(DeltaParams { instrument1, instrument2 });
        } else {
            is_valid.set(false);
        }
    });

    view! {
        <Flex>
            <Flex vertical=true align=FlexAlign::Center>
                <Label>"Инструмент 1:"</Label>
                <Select value=instrument1 default_value=params.get().instrument1.to_string()>
                    <For
                        each = move || instruments.get()
                        key = |instrument| instrument.id
                        let (instrument)
                    >
                        <option value=instrument.id.to_string()>{instrument.symbol}</option>
                    </For>
                </Select>
            </Flex>
            <Flex vertical=true align=FlexAlign::Center>
                <Label>"Инструмент 2:"</Label>
                <Select value=instrument2 default_value=params.get().instrument2.to_string()>
                    <For
                        each = move || instruments.get()
                        key = |instrument| instrument.id
                        let (instrument)
                    >
                        <option value=instrument.id.to_string()>{instrument.symbol}</option>
                    </For>
                </Select>
            </Flex>
        </Flex>
    }
}
