use crate::app::get_instruments;
use crate::client_model::ClientInstrument;
use leptos::leptos_dom::logging::console_error;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use thaw::{ConfigProvider, Flex, FlexGap, FlexJustify, Tab, TabList};
use crate::components::delta_page::DeltasPage;
use crate::components::prices_page::PricesPage;

#[component]
pub fn HomePage() -> impl IntoView {
    let instruments_resource: LocalResource<Vec<ClientInstrument>> =
        LocalResource::new(async move || {
            get_instruments().await.unwrap_or_else(|err| {
                console_error(&err.to_string());
                Vec::default()
            })
        });
    let instruments = Signal::derive(move || instruments_resource.get().unwrap_or_default());

    let active_page = RwSignal::new(String::new());

    view! {
        <ConfigProvider>
            <Flex vertical=true gap=FlexGap::Large>
                <Flex gap=FlexGap::Large justify=FlexJustify::Center class="homeTabs">
                    <TabList selected_value=active_page>
                        <Tab value="prices">"Цены"</Tab>
                        <Tab value="deltas">"Дельты"</Tab>
                    </TabList>
                </Flex>
                <PricesPage instruments is_show=Signal::derive(move || active_page.get() == "prices") />
                <DeltasPage instruments is_show=Signal::derive(move || active_page.get() == "deltas") />
            </Flex>
        </ConfigProvider>
    }
}
