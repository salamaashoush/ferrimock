use axum::{Router, http::header, response::IntoResponse, routing::get};

const BODY: &str = r#"{"id":1,"name":"John Smith","email":"john@example.com","active":true}"#;

async fn static_json() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], BODY)
}

#[tokio::main]
async fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4100);
    let app = Router::new().route("/api/static", get(static_json));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind baseline port");
    axum::serve(listener, app).await.expect("serve baseline");
}
