mod api;
mod db;
mod schedule;

use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDateTime};
use rusqlite::Connection;
use tower_http::services::{ServeDir, ServeFile};

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

#[tokio::main]
async fn main() {
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "instance/database.db".to_string());
    if let Some(dir) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(dir).expect("failed to create database directory");
    }
    let conn = Connection::open(&db_path).expect("failed to open database");
    db::migrate(&conn).expect("failed to migrate database");
    let db: api::Db = Arc::new(Mutex::new(conn));

    let app = api::router(db)
        .route_service("/", ServeFile::new("templates/index.html"))
        .nest_service("/static", ServeDir::new("static"));

    let port = std::env::var("PORT").unwrap_or_else(|_| "5000".to_string());
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind address");
    println!("Listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
