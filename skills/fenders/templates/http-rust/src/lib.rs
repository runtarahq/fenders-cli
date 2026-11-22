//! A `wasi:http` handler for Fenders: it answers every request with its
//! method, path and body size. Replace `respond` with the app.
//!
//! Build: cargo build --release --target wasm32-wasip2
//! Output: target/wasm32-wasip2/release/fenders_http_app.wasm

use wasi::http::types::{
    Fields, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

struct App;
wasi::http::proxy::export!(App);

impl wasi::exports::http::incoming_handler::Guest for App {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        let method = match request.method() {
            Method::Get => "GET".to_string(),
            Method::Post => "POST".to_string(),
            Method::Put => "PUT".to_string(),
            Method::Delete => "DELETE".to_string(),
            Method::Patch => "PATCH".to_string(),
            Method::Head => "HEAD".to_string(),
            Method::Options => "OPTIONS".to_string(),
            Method::Connect => "CONNECT".to_string(),
            Method::Trace => "TRACE".to_string(),
            Method::Other(m) => m,
        };
        let path = request.path_with_query().unwrap_or_else(|| "/".into());
        let body = read_body(request);
        let (status, content_type, text) = respond(&method, &path, &body);
        send(out, status, content_type, text.as_bytes());
        // Goes to the app's logs: `fenders logs APP`.
        println!("{method} {path} -> {status}");
    }
}

/// The app. Each request runs in a fresh instance: nothing kept in memory
/// survives to the next request.
fn respond(method: &str, path: &str, body: &[u8]) -> (u16, &'static str, String) {
    (
        200,
        "text/plain; charset=utf-8",
        format!("{method} {path} ({} bytes)\n", body.len()),
    )
}

fn read_body(request: IncomingRequest) -> Vec<u8> {
    let mut out = Vec::new();
    let Ok(body) = request.consume() else {
        return out;
    };
    if let Ok(stream) = body.stream() {
        // Until the stream is closed (an error), in chunks of up to 64 KiB.
        while let Ok(chunk) = stream.blocking_read(64 * 1024) {
            out.extend_from_slice(&chunk);
        }
    }
    out
}

fn send(out: ResponseOutparam, status: u16, content_type: &str, bytes: &[u8]) {
    let headers = Fields::new();
    headers
        .set("content-type", &[content_type.as_bytes().to_vec()])
        .unwrap();
    let response = OutgoingResponse::new(headers);
    response.set_status_code(status).unwrap();
    let body = response.body().unwrap();
    ResponseOutparam::set(out, Ok(response));
    let stream = body.write().unwrap();
    // blocking_write_and_flush takes at most 4096 bytes at a time.
    for chunk in bytes.chunks(4096) {
        stream.blocking_write_and_flush(chunk).unwrap();
    }
    drop(stream);
    OutgoingBody::finish(body, None).unwrap();
}
