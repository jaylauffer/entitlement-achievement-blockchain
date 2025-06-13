use actix_web::{App, HttpServer, web};
use std::sync::RwLock;

use rust_blockchain::player_profile::profile_service::PlayerProfileService;
use rust_blockchain::ledger_storage::FileTopicLedgerStorage;
use rust_blockchain::api;

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

