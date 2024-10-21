use axum::{routing::get, Router};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // build our application with a route
    let app = Router::new().route("/", get(handler));

    // run it
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> &'static str {
    "Hello, World!"
}