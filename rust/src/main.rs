use actix_web::{App, HttpServer, web};
use std::sync::Mutex;

use rust_blockchain::player_profile::profile_service::PlayerProfileService;
use rust_blockchain::api;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let service = web::Data::new(Mutex::new(PlayerProfileService::new()));

    HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .configure(api::init_routes)
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
