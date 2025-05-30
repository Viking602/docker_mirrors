mod config;
mod handlers;
mod models;
mod services;
mod utils;

use actix_web::{web, App, HttpServer};
use log::info;

use crate::config::RegistryConfig;
use crate::handlers::registry::handle_registry_request;
use crate::models::app_state::AppState;
use crate::services::proxy::ProxyService;
use crate::utils::logging::init_logger;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logger
    init_logger();

    // Create registry configuration
    let registry_config = RegistryConfig::default();

    // Create proxy service
    let proxy_service = ProxyService::new(registry_config);

    // Create application state
    let app_state = AppState::new(proxy_service);

    info!("Starting Docker registry mirror server on 0.0.0.0:8080");

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.proxy_service.clone())
            .route("/{registry}/{path:.*}", web::get().to(handle_registry_request))
            .route("/{registry}/{path:.*}", web::post().to(handle_registry_request))
            .route("/{registry}/{path:.*}", web::put().to(handle_registry_request))
            .route("/{registry}/{path:.*}", web::delete().to(handle_registry_request))
            .route("/{registry}/{path:.*}", web::head().to(handle_registry_request))
            .route("/{registry}/{path:.*}", web::patch().to(handle_registry_request))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
