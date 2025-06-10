use actix_web::{App, HttpServer, web};
use std::sync::Mutex;
use std::env;

use rust_blockchain::player_profile::profile_service::PlayerProfileService;
use rust_blockchain::api;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let service = web::Data::new(Mutex::new(PlayerProfileService::new()));

    let bind_ip = env::var("BIND_IP").unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_port: u16 = env::var("BIND_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .configure(api::init_routes)
    })
    .bind((bind_ip.as_str(), bind_port))?
    .run()
    .await
}
