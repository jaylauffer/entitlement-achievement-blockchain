use actix_web::{App, HttpServer, web};
use std::sync::RwLock;
use std::env;

use rust_blockchain::player_profile::profile_service::PlayerProfileService;
use rust_blockchain::ledger_storage::FileTopicLedgerStorage;
use rust_blockchain::sled_ledger_storage::SledLedgerStorage;
use rust_blockchain::api;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let storage_backend = env::var("LEDGER_BACKEND").unwrap_or_else(|_| "file".to_string());
    let storage: Box<dyn rust_blockchain::ledger_storage::LedgerStorage + Send + Sync> = if storage_backend == "sled" {
        let path = env::var("LEDGER_DB_PATH").unwrap_or_else(|_| "ledger_db".to_string());
        Box::new(SledLedgerStorage::new(path))
    } else {
        Box::new(FileTopicLedgerStorage::new("player_logs"))
    };
    let service = web::Data::new(RwLock::new(PlayerProfileService::new(storage)));

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
