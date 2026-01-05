use std::rc::Rc;
use std::sync::Mutex;
use crate::app::get_deltas;
use crate::client_model::{ClientInstrument, DeltaParams, MainParams};
use crate::components::delta_chart::DeltaChart;
use crate::components::delta_control::DeltaControl;
use crate::components::main_control::MainControl;
use leptos::leptos_dom::logging::console_error;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use thaw::Flex;
use crate::client_model::process_chart_data;

#[component]
pub fn DeltasPage(instruments: Signal<Vec<ClientInstrument>>, is_show: Signal<bool>) -> impl IntoView {
    let main_params = RwSignal::new(MainParams::default());
    let params = RwSignal::new(DeltaParams::default());
    let is_req_allowed = RwSignal::new(false);

    let params_buf = Rc::new(Mutex::new(DeltaParams::default()));
    let params_clone = params_buf.clone();
    Effect::new(move || {
        let new_params = params.get();
        let mut params = params_clone.lock().unwrap();
        params.instrument1 = new_params.instrument1;
        params.instrument2 = new_params.instrument2;
    });

    let action = Action::new(|params: &(MainParams, DeltaParams)| {
        let params = params.clone();
        async move {
            get_deltas(params).await.unwrap_or_else(|err| {
                console_error(&err.to_string());
                Default::default()
            })
        }
    });
    let params_clone = params_buf.clone();
    Effect::new(move || {
        let main_params = main_params.get();
        let params = params_clone.lock().unwrap().clone();
        if params.instrument1 != 0 && params.instrument2 != 0 {
            action.dispatch((main_params, params));
        }
    });
    let deltas = Signal::derive(move || action.value().get().unwrap_or_default());

    view! {
        <Show when=move || is_show.get()>
            <Flex vertical=true>
                <Flex>
                    <MainControl
                        params=main_params
                        is_loading=Signal::derive(move || action.pending().get())
                        is_req_disabled=Signal::derive(move || !is_req_allowed.get())
                          />
                    <DeltaControl instruments params is_valid=is_req_allowed />
                </Flex>
                <Show when=move || !deltas.get().is_empty()>
                    <DeltaChart
                        data=Signal::derive(move || process_chart_data(&deltas.get())) />
                </Show>
            </Flex>
        </Show>
    }
}
