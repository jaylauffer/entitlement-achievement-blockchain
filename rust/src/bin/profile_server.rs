use actix_web::{web, App, HttpServer};
use std::sync::RwLock;

use loadngo_eab::api;
use loadngo_eab::ledger_storage::FileTopicLedgerStorage;
use loadngo_eab::player_profile::profile_service::PlayerProfileService;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let storage = FileTopicLedgerStorage::new("player_logs");
    let service = web::Data::new(RwLock::new(PlayerProfileService::new(Box::new(storage))));

    HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .configure(api::init_routes)
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
