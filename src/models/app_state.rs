use crate::services::proxy::ProxyService;
use actix_web::web;

pub struct AppState {
    pub proxy_service: web::Data<ProxyService>,
}

impl AppState {
    pub fn new(proxy_service: ProxyService) -> Self {
        Self {
            proxy_service: web::Data::new(proxy_service),
        }
    }
}