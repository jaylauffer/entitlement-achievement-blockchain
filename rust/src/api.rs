use crate::achievement_registry::{AchievementDefinition, AchievementRegistry};
use crate::blockchain::TransactionData;
use crate::concept_registry::ConceptRegistry;
use crate::entitlement_registry::{EntitlementDefinition, EntitlementRegistry};
use crate::hd::BitVec;
use crate::identity::{exchange_identity, player_id_from_session, IdentityError};
use crate::player_profile::profile_service::PlayerProfileService;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::RwLock;
use std::{collections::HashMap, env, fs};

static DEVELOPER_TOKENS: Lazy<Vec<(String, String)>> = Lazy::new(|| {
    if let Ok(path) = env::var("DEVELOPER_TOKENS_FILE") {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&contents) {
                return map.into_iter().collect();
            }
        }
    }
    if let Ok(var) = env::var("DEVELOPER_TOKENS") {
        return var
            .split(',')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, ':');
                match (parts.next(), parts.next()) {
                    (Some(d), Some(t)) => Some((d.trim().to_string(), t.trim().to_string())),
                    _ => None,
                }
            })
            .collect();
    }
    vec![
        ("dev1".to_string(), "token1".to_string()),
        ("dev2".to_string(), "token2".to_string()),
    ]
});

fn authorized(req: &HttpRequest) -> Option<String> {
    match req.headers().get("Authorization") {
        Some(value) => {
            let val = value.to_str().ok()?;
            for (dev, token) in DEVELOPER_TOKENS.iter() {
                if val == format!("Bearer {}", token) {
                    return Some(dev.to_string());
                }
            }
            None
        }
        None => None,
    }
}

fn player_token(req: &HttpRequest) -> Option<String> {
    match req.headers().get("Authorization") {
        Some(value) => {
            let val = value.to_str().ok()?;
            val.strip_prefix("Bearer ").map(|token| token.to_string())
        }
        None => None,
    }
}

fn player_id_from_request(req: &HttpRequest) -> Option<String> {
    let token = player_token(req)?;
    player_id_from_session(&token)
}

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/identity/exchange").route(web::post().to(exchange_identity_token)))
        .service(web::resource("/profiles").route(web::post().to(create_profile)))
        .service(web::resource("/profiles/{id}").route(web::get().to(get_profile)))
        .service(web::resource("/profiles/{id}/rewards").route(web::get().to(get_rewards)))
        .service(web::resource("/profiles/{id}/dimensions").route(web::post().to(set_dimensions)))
        .service(web::resource("/concepts").route(web::post().to(add_concept)))
        .service(
            web::resource("/concepts/{developer}/{game}/{concept}")
                .route(web::get().to(get_concept)),
        )
        .service(
            web::resource("/profiles/{id}/concepts").route(web::post().to(add_concept_to_profile)),
        )
        .service(web::resource("/achievements").route(web::post().to(add_achievement)))
        .service(
            web::resource("/profiles/{id}/achievements")
                .route(web::post().to(award_achievement_to_profile)),
        )
        .service(web::resource("/entitlements").route(web::post().to(add_entitlement)))
        .service(
            web::resource("/profiles/{id}/entitlements")
                .route(web::post().to(award_entitlement_to_profile)),
        );
}

#[derive(Deserialize)]
struct CreateProfileData {
    name: String,
}

async fn create_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    info: web::Json<CreateProfileData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    match svc.create_profile(&player_id, &info.name) {
        Ok(profile) => HttpResponse::Ok().json(profile),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

async fn get_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    let svc = match service.read() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }
    if let Some(profile) = svc.get_profile(&path.into_inner()) {
        HttpResponse::Ok().json(profile)
    } else {
        HttpResponse::NotFound().finish()
    }
}

async fn get_rewards(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }

    let svc = match service.read() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    if let Some(rewards) = svc.get_reward_state(&path) {
        HttpResponse::Ok().json(rewards)
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(serde::Serialize)]
struct AwardReceipt {
    player_id: String,
    transaction_id: String,
    transaction_type: String,
    timestamp: String,
    data_hash: String,
    block_hash: String,
    details: TransactionData,
}

fn latest_award_receipt(svc: &PlayerProfileService, player_id: &str) -> Option<AwardReceipt> {
    for block in svc.ledger.chain.iter().rev() {
        for tx in block.transactions.iter().rev() {
            if tx.player_id == player_id {
                return Some(AwardReceipt {
                    player_id: tx.player_id.clone(),
                    transaction_id: tx.transaction_id.clone(),
                    transaction_type: tx.transaction_type.clone(),
                    timestamp: tx.timestamp.clone(),
                    data_hash: tx.data_hash.clone(),
                    block_hash: block.block_hash.clone(),
                    details: tx.details.clone(),
                });
            }
        }
    }
    None
}

#[derive(Deserialize)]
struct DimensionsData {
    lanes: Vec<u64>,
    dim: usize,
}

async fn set_dimensions(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
    info: web::Json<DimensionsData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }
    let vec = BitVec {
        dim: info.dim,
        lanes: info.lanes.clone(),
    };
    if svc.set_vector(&path, vec).is_ok() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct ConceptData {
    developer: String,
    game: String,
    concept: String,
    dim: Option<usize>,
}

async fn add_concept(req: HttpRequest, info: web::Json<ConceptData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let mut reg = ConceptRegistry::load("concept_registry.json").unwrap_or_default();
    let key = format!("{}:{}:{}", info.developer, info.game, info.concept);
    let dim = info
        .dim
        .unwrap_or(crate::player_profile::profile_service::DEFAULT_DIM);
    let vec = match reg.get(&key) {
        Some(v) => v.clone(),
        None => {
            let v = BitVec::seed(&key, dim);
            reg.insert(key.clone(), v.clone());
            let _ = reg.save("concept_registry.json");
            v
        }
    };
    HttpResponse::Ok().json(vec)
}

async fn get_concept(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
) -> impl Responder {
    if authorized(&req).as_deref() != Some(&path.0) {
        return HttpResponse::Unauthorized().finish();
    }
    let reg = ConceptRegistry::load("concept_registry.json").unwrap_or_default();
    let key = format!("{}:{}:{}", path.0, path.1, path.2);
    if let Some(vec) = reg.get(&key) {
        HttpResponse::Ok().json(vec)
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct AssignConceptData {
    developer: String,
    game: String,
    concept: String,
}

async fn add_concept_to_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
    info: web::Json<AssignConceptData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }
    let reg = ConceptRegistry::load("concept_registry.json").unwrap_or_default();
    let key = format!("{}:{}:{}", info.developer, info.game, info.concept);
    if let Some(vec) = reg.get(&key) {
        let mut svc = match service.write() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        if svc.merge_vector(&path, vec).is_ok() {
            HttpResponse::Ok().finish()
        } else {
            HttpResponse::NotFound().finish()
        }
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct AchievementDefData {
    developer: String,
    game: String,
    achievement_id: String,
    version: u32,
    name: String,
    description: String,
}

async fn add_achievement(req: HttpRequest, info: web::Json<AchievementDefData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let mut reg = AchievementRegistry::load("achievement_registry.json").unwrap_or_default();
    let def = AchievementDefinition {
        developer: info.developer.clone(),
        game: info.game.clone(),
        achievement_id: info.achievement_id.clone(),
        version: info.version,
        name: info.name.clone(),
        description: info.description.clone(),
    };
    reg.insert(def);
    let _ = reg.save("achievement_registry.json");
    HttpResponse::Ok().finish()
}

#[derive(Deserialize)]
struct AwardData {
    developer: String,
    game: String,
    achievement_id: String,
    version: u32,
}

async fn award_achievement_to_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
    info: web::Json<AwardData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }
    let reg = AchievementRegistry::load("achievement_registry.json").unwrap_or_default();
    if let Some(def) = reg.get(
        &info.developer,
        &info.game,
        &info.achievement_id,
        info.version,
    ) {
        let mut svc = match service.write() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        match svc.award_achievement(&path, def) {
            Ok(_) => {
                if let Some(receipt) = latest_award_receipt(&svc, &path) {
                    HttpResponse::Ok().json(receipt)
                } else {
                    HttpResponse::Ok().finish()
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
            Err(_) => HttpResponse::InternalServerError().finish(),
        }
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct EntitlementDefData {
    developer: String,
    game: String,
    entitlement_id: String,
    version: u32,
    item_type: String,
    item_id: String,
    description: String,
}

async fn add_entitlement(req: HttpRequest, info: web::Json<EntitlementDefData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let mut reg = EntitlementRegistry::load("entitlement_registry.json").unwrap_or_default();
    let def = EntitlementDefinition {
        developer: info.developer.clone(),
        game: info.game.clone(),
        entitlement_id: info.entitlement_id.clone(),
        version: info.version,
        item_type: info.item_type.clone(),
        item_id: info.item_id.clone(),
        description: info.description.clone(),
    };
    reg.insert(def);
    let _ = reg.save("entitlement_registry.json");
    HttpResponse::Ok().finish()
}

#[derive(Deserialize)]
struct GrantEntitlementData {
    developer: String,
    game: String,
    entitlement_id: String,
    version: u32,
    quantity: u32,
    expiration_date: Option<String>,
}

async fn award_entitlement_to_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
    info: web::Json<GrantEntitlementData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }
    let reg = EntitlementRegistry::load("entitlement_registry.json").unwrap_or_default();
    if let Some(def) = reg.get(
        &info.developer,
        &info.game,
        &info.entitlement_id,
        info.version,
    ) {
        let mut svc = match service.write() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        match svc.award_entitlement(&path, def, info.quantity, info.expiration_date.clone()) {
            Ok(_) => {
                if let Some(receipt) = latest_award_receipt(&svc, &path) {
                    HttpResponse::Ok().json(receipt)
                } else {
                    HttpResponse::Ok().finish()
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
            Err(_) => HttpResponse::InternalServerError().finish(),
        }
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct ExchangeIdentityRequest {
    provider: String,
    token: String,
}

#[derive(serde::Serialize)]
struct ExchangeIdentityResponse {
    access_token: String,
    player_id: String,
    is_new_player: bool,
}

async fn exchange_identity_token(info: web::Json<ExchangeIdentityRequest>) -> impl Responder {
    match exchange_identity(&info.provider, &info.token) {
        Ok(result) => HttpResponse::Ok().json(ExchangeIdentityResponse {
            access_token: result.access_token,
            player_id: result.player_id,
            is_new_player: result.is_new_player,
        }),
        Err(IdentityError::UnsupportedProvider) => {
            HttpResponse::BadRequest().body("unsupported provider")
        }
        Err(IdentityError::InvalidToken) => HttpResponse::Unauthorized().body("invalid token"),
        Err(IdentityError::StorageError) => HttpResponse::InternalServerError().finish(),
    }
}
