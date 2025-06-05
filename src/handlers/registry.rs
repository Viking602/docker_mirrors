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

    // 构建查询字符串
    let query_string = if !query.is_empty() {
        format!("?{}", query.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&"))
    } else {
        String::new()
    };

    // 收集请求头
    let headers = collect_headers(&req);
    
    // 收集请求体
    let body_bytes = match collect_body(body).await {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        Ok(_) => None,
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return HttpResponse::InternalServerError().body(format!("Failed to read request body: {}", e));
        }
    };

    // 转发请求
    match proxy_service.forward_request(
        &registry_key,
        &path_str,
        if query_string.is_empty() { None } else { Some(&query_string) },
        headers,
        body_bytes,
        req.method().as_str(),
    ).await {
        Ok(response) => {
            let status = response.status();
            info!("Upstream response status: {}", status);
            
            let mut builder = HttpResponse::build(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));

            // 复制响应头
            for (key, value) in response.headers() {
                if let Ok(header_name) = actix_web::http::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                    if let Ok(header_value) = actix_web::http::header::HeaderValue::from_bytes(value.as_bytes()) {
                        builder.append_header((header_name, header_value));
                    }
                }
            }

            // 处理响应体
            match response.bytes().await {
                Ok(bytes) => {
                    if status.is_success() {
                        builder.body(bytes)
                    } else {
                        let error_body = if bytes.is_empty() {
                            format!("Upstream error: {}", status)
                        } else {
                            String::from_utf8_lossy(&bytes).to_string()
                        };
                        builder.body(error_body)
                    }
                },
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

fn collect_headers(req: &HttpRequest) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (key, value) in req.headers() {
        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
            if let Ok(header_value) = reqwest::header::HeaderValue::from_str(value.to_str().unwrap_or_default()) {
                headers.insert(header_name, header_value);
            }
        }
    }
    headers
}

async fn collect_body(mut body: web::Payload) -> Result<Bytes, Box<dyn std::error::Error>> {
    let mut bytes = web::BytesMut::new();
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk?);
    }
    Ok(bytes.freeze())
}
