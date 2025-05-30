use actix_web::{web, HttpRequest, HttpResponse, http::StatusCode};
use bytes::Bytes;
use futures::StreamExt;
use log::{error, info};
use crate::services::proxy::ProxyService;

pub async fn handle_registry_request(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    query: web::Query<std::collections::HashMap<String, String>>,
    body: web::Payload,
    proxy_service: web::Data<ProxyService>,
) -> HttpResponse {
    let (registry_key, path_tail) = path.into_inner();
    let path_str = format!("/{}", path_tail);

    info!("Received request: {} {} for registry: {}", req.method(), req.uri(), registry_key);

    // Convert query parameters to string
    let query_string = if !query.is_empty() {
        let query_pairs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        format!("?{}", query_pairs.join("&"))
    } else {
        String::new()
    };

    // Get request method
    let method = req.method().as_str();

    // Get headers
    let mut headers = reqwest::header::HeaderMap::new();
    info!("Request headers:");
    for (key, value) in req.headers() {
        info!("  {}: {}", key, value.to_str().unwrap_or_default());
        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
            if let Ok(header_value) = reqwest::header::HeaderValue::from_str(value.to_str().unwrap_or_default()) {
                headers.insert(header_name, header_value);
            }
        }
    }

    // Collect body
    let body_bytes = match collect_body(body).await {
        Ok(bytes) => {
            if bytes.is_empty() {
                None
            } else {
                Some(bytes)
            }
        },
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return HttpResponse::InternalServerError().body(format!("Failed to read request body: {}", e));
        }
    };

    // Forward request
    match proxy_service.forward_request(
        &registry_key,
        &path_str,
        if query_string.is_empty() { None } else { Some(&query_string) },
        headers,
        body_bytes,
        method,
    ).await {
        Ok(response) => {
            let status = response.status();
            let mut builder = HttpResponse::build(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK));

            // Copy headers
            for (key, value) in response.headers() {
                builder.append_header((key.as_str(), value.to_str().unwrap_or_default()));
            }

            // Get response body
            match response.bytes().await {
                Ok(bytes) => builder.body(bytes),
                Err(e) => {
                    error!("Failed to read response body: {}", e);
                    HttpResponse::InternalServerError().body(format!("Failed to read response body: {}", e))
                }
            }
        },
        Err(e) => {
            error!("Failed to forward request: {}", e);
            HttpResponse::InternalServerError().body(format!("Failed to forward request: {}", e))
        }
    }
}

async fn collect_body(mut body: web::Payload) -> Result<Bytes, Box<dyn std::error::Error>> {
    let mut bytes = web::BytesMut::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes.freeze())
}
