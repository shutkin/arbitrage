use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment, WildcardSegment,
};
use crate::client_model::{PricesParams, ClientInstrument, PricesResponse, MainParams, DeltaParams, TimedValues};
use crate::components::home_page::HomePage;

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/viewer.css" />
        <Title text="Arbitrage" />
        <Router>
            <main>
                <Routes fallback=move || "Not found.">
                    <Route path=StaticSegment("") view=HomePage />
                    <Route path=WildcardSegment("any") view=NotFound />
                </Routes>
            </main>
        </Router>
    }
}

#[server]
pub async fn get_instruments() -> Result<Vec<ClientInstrument>, ServerFnError> {
    let api = expect_context::<crate::api::API>();
    match api.get_instruments().await {
        Ok(data) => Ok(data),
        Err(err) => Err(ServerFnError::ServerError(err.to_string())),
    }
}

#[server(input=Json)]
pub async fn get_prices(params: (MainParams, PricesParams)) -> Result<PricesResponse, ServerFnError> {
    let api = expect_context::<crate::api::API>();
    match api.get_prices(params.0, params.1).await {
        Ok(data) => Ok(data),
        Err(err) => Err(ServerFnError::ServerError(err.to_string())),
    }
}

#[server(input=Json)]
pub async fn get_deltas(params: (MainParams, DeltaParams)) -> Result<Vec<TimedValues>, ServerFnError> {
    let api = expect_context::<crate::api::API>();
    match api.get_deltas(params.0, params.1).await {
        Ok(data) => Ok(data),
        Err(err) => Err(ServerFnError::ServerError(err.to_string())),
    }
}

/// 404 - Not Found
#[component]
fn NotFound() -> impl IntoView {
    // set an HTTP status code 404
    // this is feature gated because it can only be done during
    // initial server-side rendering
    // if you navigate to the 404 page subsequently, the status
    // code will not be set because there is not a new HTTP request
    // to the server
    #[cfg(feature = "ssr")]
    {
        // this can be done inline because it's synchronous
        // if it were async, we'd use a server function
        let resp = expect_context::<leptos_actix::ResponseOptions>();
        resp.set_status(actix_web::http::StatusCode::NOT_FOUND);
    }

    view! { <h1>"Not Found"</h1> }
}