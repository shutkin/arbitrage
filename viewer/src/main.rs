use thaw::ssr::SSRMountStyleProvider;
use crate::api::API;

pub mod app;
#[cfg(feature = "ssr")]
mod api;
mod components;
mod client_model;

#[cfg(feature = "ssr")]
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    use actix_files::Files;
    use actix_web::*;
    use leptos::prelude::*;
    use leptos::config::get_configuration;
    use leptos_meta::MetaTags;
    use leptos_actix::{generate_route_list, LeptosRoutes};
    use simplelog::{SimpleLogger, LevelFilter};
    use crate::app::*;

    SimpleLogger::init(LevelFilter::Info, simplelog::Config::default()).ok();
    dotenv::dotenv().ok();
    let db_url = std::env::var("DB_URL").expect("DB_URL is not set");
    let api = API::new(&db_url).await.map_err(std::io::Error::other)?;

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;

    HttpServer::new(move || {
        let api_1 = api.clone();
        let api_2 = api.clone();

        // Generate the list of routes in your Leptos App
        let routes = generate_route_list(App);
        let leptos_options = &conf.leptos_options;
        let site_root = leptos_options.site_root.clone().to_string();

        println!("listening on http://{}", &addr);

        App::new()
            // serve JS/WASM/CSS from `pkg`
            .service(Files::new("/pkg", format!("{site_root}/pkg")))
            // serve other assets from the `assets` directory
            .service(Files::new("/assets", &site_root))
            // serve the favicon from /favicon.ico
            .service(favicon)
            .route("/api/{tail:.*}", leptos_actix::handle_server_fns_with_context(move || provide_context(api_1.clone())))
            .leptos_routes_with_context(routes, move || provide_context(api_2.clone()), {
                let leptos_options = leptos_options.clone();
                move || {
                    let leptos_options = leptos_options.clone();
                    view! {
                        <SSRMountStyleProvider>
                            <!DOCTYPE html>
                            <html lang="en">
                                <head>
                                    <meta charset="utf-8" />
                                    <meta
                                        name="viewport"
                                        content="width=device-width, initial-scale=1"
                                    />
                                    <AutoReload options=leptos_options.clone() />
                                    <HydrationScripts options=leptos_options.clone() />
                                    <MetaTags />
                                </head>
                                <body>
                                    <App />
                                </body>
                            </html>
                        </SSRMountStyleProvider>
                    }
                }
            })
            .app_data(web::Data::new(leptos_options.to_owned()))
        //.wrap(middleware::Compress::default())
    })
        .bind(&addr)?
        .run()
        .await
}

#[cfg(feature = "ssr")]
#[actix_web::get("favicon.ico")]
async fn favicon(
    leptos_options: actix_web::web::Data<leptos::config::LeptosOptions>,
) -> actix_web::Result<actix_files::NamedFile> {
    let leptos_options = leptos_options.into_inner();
    let site_root = &leptos_options.site_root;
    Ok(actix_files::NamedFile::open(format!(
        "{site_root}/favicon.ico"
    ))?)
}

#[cfg(not(any(feature = "ssr", feature = "csr")))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
    // see optional feature `csr` instead
}

#[cfg(all(not(feature = "ssr"), feature = "csr"))]
pub fn main() {
    // a client-side main function is required for using `trunk serve`
    // prefer using `cargo leptos serve` instead
    // to run: `trunk serve --open --features csr`
    use crate::app::*;

    console_error_panic_hook::set_once();

    leptos::mount_to_body(App);
}