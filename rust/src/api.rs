use actix_web::{web, HttpResponse, Responder, HttpRequest};
use crate::player_profile::profile_service::PlayerProfileService;
use std::sync::RwLock;
use crate::hd::BitVec;
use crate::concept_registry::ConceptRegistry;
use crate::achievement_registry::{AchievementRegistry, AchievementDefinition};
use crate::entitlement_registry::{EntitlementRegistry, EntitlementDefinition};
use serde::Deserialize;
use once_cell::sync::Lazy;
use std::{collections::HashMap, fs, env};

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

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/profiles")
            .route(web::post().to(create_profile))
    )
    .service(
        web::resource("/profiles/{id}")
            .route(web::get().to(get_profile))
    )
    .service(
        web::resource("/profiles/{id}/dimensions")
            .route(web::post().to(set_dimensions))
    )
    .service(
        web::resource("/concepts")
            .route(web::post().to(add_concept))
    )
    .service(
        web::resource("/concepts/{developer}/{game}/{concept}")
            .route(web::get().to(get_concept))
    )
    .service(
        web::resource("/profiles/{id}/concepts")
            .route(web::post().to(add_concept_to_profile))
    )
    .service(
        web::resource("/achievements")
            .route(web::post().to(add_achievement))
    )
    .service(
        web::resource("/profiles/{id}/achievements")
            .route(web::post().to(award_achievement_to_profile))
    )
    .service(
        web::resource("/entitlements")
            .route(web::post().to(add_entitlement))
    )
    .service(
        web::resource("/profiles/{id}/entitlements")
            .route(web::post().to(award_entitlement_to_profile))
    );
}

#[derive(Deserialize)]
struct CreateProfileData {
    name: String,
}

async fn create_profile(service: web::Data<RwLock<PlayerProfileService>>, req: HttpRequest, info: web::Json<CreateProfileData>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    match svc.create_profile("player", &info.name) {
        Ok(profile) => HttpResponse::Ok().json(profile),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

async fn get_profile(service: web::Data<RwLock<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let svc = match service.read() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    if let Some(profile) = svc.get_profile(&path.into_inner()) {
        HttpResponse::Ok().json(profile)
    } else {
        HttpResponse::NotFound().finish()
    }
}

#[derive(Deserialize)]
struct DimensionsData {
    lanes: Vec<u64>,
    dim: usize,
}

async fn set_dimensions(service: web::Data<RwLock<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>, info: web::Json<DimensionsData>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    let vec = BitVec { dim: info.dim, lanes: info.lanes.clone() };
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
    let dim = info.dim.unwrap_or(crate::player_profile::profile_service::DEFAULT_DIM);
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

async fn get_concept(req: HttpRequest, path: web::Path<(String, String, String)>) -> impl Responder {
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

async fn add_concept_to_profile(service: web::Data<RwLock<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>, info: web::Json<AssignConceptData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
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

async fn award_achievement_to_profile(service: web::Data<RwLock<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>, info: web::Json<AwardData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let reg = AchievementRegistry::load("achievement_registry.json").unwrap_or_default();
    if let Some(def) = reg.get(&info.developer, &info.game, &info.achievement_id, info.version) {
        let mut svc = match service.write() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        match svc.award_achievement(&path, def) {
            Ok(_) => HttpResponse::Ok().finish(),
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
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let reg = EntitlementRegistry::load("entitlement_registry.json").unwrap_or_default();
    if let Some(def) = reg.get(&info.developer, &info.game, &info.entitlement_id, info.version) {
        let mut svc = match service.write() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        match svc.award_entitlement(&path, def, info.quantity, info.expiration_date.clone()) {
            Ok(_) => HttpResponse::Ok().finish(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
            Err(_) => HttpResponse::InternalServerError().finish(),
        }
    } else {
        HttpResponse::NotFound().finish()
    }
}

