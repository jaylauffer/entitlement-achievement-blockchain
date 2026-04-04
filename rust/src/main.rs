use actix_web::{web, App, HttpServer};
use std::env;
use std::sync::RwLock;

use rust_blockchain::api;
use rust_blockchain::ledger_storage::FileTopicLedgerStorage;
use rust_blockchain::player_profile::profile_service::PlayerProfileService;
use rust_blockchain::qcoin_ledger_storage::QCoinLedgerStorage;
use rust_blockchain::sled_ledger_storage::SledLedgerStorage;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let storage_backend = env::var("LEDGER_BACKEND").unwrap_or_else(|_| "file".to_string());
    let storage: Box<dyn rust_blockchain::ledger_storage::LedgerStorage + Send + Sync> =
        if storage_backend == "sled" {
            let path = env::var("LEDGER_DB_PATH").unwrap_or_else(|_| "ledger_db".to_string());
            Box::new(SledLedgerStorage::new(path))
        } else if storage_backend == "qcoin" {
            let topic_path =
                env::var("LEDGER_TOPICS_PATH").unwrap_or_else(|_| "player_logs".to_string());
            let qcoin_state_path = env::var("QCOIN_STATE_PATH")
                .unwrap_or_else(|_| "qcoin_chain_state.json".to_string());
            Box::new(QCoinLedgerStorage::new(topic_path, qcoin_state_path))
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
