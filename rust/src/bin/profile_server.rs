use actix_web::{web, App, HttpServer};

use loadngo_eab::api;
use loadngo_eab::eab_node::StaticStatusProvider;
use loadngo_eab::ledger_storage::FileTopicLedgerStorage;
use loadngo_eab::runtime::EabRuntime;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let storage = FileTopicLedgerStorage::new("player_logs");
    let runtime = web::Data::new(EabRuntime::new(
        Box::new(storage),
        std::sync::Arc::new(StaticStatusProvider::new("file")),
        None,
    )?);

    HttpServer::new(move || {
        App::new()
            .app_data(runtime.clone())
            .configure(api::init_routes)
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
