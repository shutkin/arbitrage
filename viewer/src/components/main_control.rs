use crate::client_model::MainParams;
use chrono::{Local, NaiveDateTime, TimeZone};
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use std::rc::Rc;
use std::sync::Mutex;
use thaw::{Button, ButtonAppearance, DatePicker, Flex, FlexAlign, Label, Select, TimePicker};

#[component]
pub fn MainControl(
    params: RwSignal<MainParams>,
    is_loading: Signal<bool>,
    is_req_disabled: Signal<bool>,
) -> impl IntoView {
    let date_value = RwSignal::new(Local::now().date_naive());
    let time_value = RwSignal::new(Local::now().time());
    let interval = RwSignal::new(15_i16);

    let params_buf = Rc::new(Mutex::new(MainParams::default()));

    let params_buf_clone = params_buf.clone();
    Effect::new(move || {
        let mut params = params_buf_clone.lock().unwrap();
        params.interval_minutes = interval.get();
        let local = NaiveDateTime::new(date_value.get(), time_value.get());
        let datetime = Local.from_local_datetime(&local).unwrap();
        params.date_time = datetime.into();
    });

    let params_buf_clone = params_buf.clone();
    let on_req = move |_| {
        params.set(params_buf_clone.lock().unwrap().clone());
    };

    view! {
        <div style="display: flex; gap: 0.5rem">
            <Button on:click=on_req
                loading=is_loading
                class="controlPanelButton"
                disabled=is_req_disabled
                appearance=ButtonAppearance::Primary>
                "Показать"
            </Button>
            <Flex vertical=true>
                <DatePicker value=date_value />
                <TimePicker value=time_value />
            </Flex>
            <Flex vertical=true align=FlexAlign::Center>
                <Label>"Период:"</Label>
                <Select
                    default_value="15"
                    on:change=move |ev| {
                        let value = event_target_value(&ev);
                        interval.set(value.parse().unwrap_or_default());
                    }>
                    <option value="15">"15 минут"</option>
                    <option value="30">"полчаса"</option>
                    <option value="60">"час"</option>
                    <option value="120">"два часа"</option>
                    <option value="240">"четыре часа"</option>
                </Select>
            </Flex>
        </div>
    }
}
