use actix_web::{web, App, HttpServer};
use std::env;
use std::sync::RwLock;

use loadngo_eab::api;
use loadngo_eab::eab_node::EabNodeRuntime;
use loadngo_eab::ledger_storage::FileTopicLedgerStorage;
use loadngo_eab::player_profile::profile_service::PlayerProfileService;
use loadngo_eab::qcoin_ledger_storage::QCoinLedgerStorage;
use loadngo_eab::sled_ledger_storage::SledLedgerStorage;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let storage_backend = env::var("LEDGER_BACKEND").unwrap_or_else(|_| "file".to_string());
    let storage: Box<dyn loadngo_eab::ledger_storage::LedgerStorage + Send + Sync> =
        if storage_backend == "sled" {
            let path = env::var("LEDGER_DB_PATH").unwrap_or_else(|_| "ledger_db".to_string());
            Box::new(SledLedgerStorage::new(path))
        } else if storage_backend == "qcoin" {
            let topic_path =
                env::var("LEDGER_TOPICS_PATH").unwrap_or_else(|_| "player_logs".to_string());
            let qcoin_outbox_path = env::var("QCOIN_OUTBOX_PATH")
                .or_else(|_| env::var("QCOIN_STATE_PATH"))
                .unwrap_or_else(|_| "qcoin_anchor_outbox.json".to_string());
            Box::new(QCoinLedgerStorage::new(topic_path, qcoin_outbox_path))
        } else {
            Box::new(FileTopicLedgerStorage::new("player_logs"))
        };
    let service = web::Data::new(RwLock::new(PlayerProfileService::new(storage)));

    let bind_ip = env::var("BIND_IP").unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_port: u16 = env::var("BIND_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let _node_runtime = match EabNodeRuntime::start_from_env(bind_ip.as_str(), bind_port) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Failed to start EAB node transport: {err}");
            None
        }
    };

    HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .configure(api::init_routes)
    })
    .bind((bind_ip.as_str(), bind_port))?
    .run()
    .await
}
