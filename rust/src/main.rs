use actix_web::{web, App, HttpServer};
use std::env;

use loadngo_eab::api;
use loadngo_eab::runtime::EabRuntime;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let bind_ip = env::var("BIND_IP").unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_port: u16 = env::var("BIND_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let runtime = web::Data::new(EabRuntime::from_env(bind_ip.as_str(), bind_port)?);

    HttpServer::new(move || {
        App::new()
            .app_data(runtime.clone())
            .configure(api::init_routes)
    })
    .bind((bind_ip.as_str(), bind_port))?
    .run()
    .await
}
