use actix_web::{web, HttpResponse, Responder, HttpRequest};
use crate::player_profile::profile_service::PlayerProfileService;
use crate::hd::BitVec;
use crate::concept_registry::ConceptRegistry;
use serde::Deserialize;

const DEVELOPER_TOKENS: &[(&str, &str)] = &[
    ("dev1", "token1"),
    ("dev2", "token2"),
];

fn authorized(req: &HttpRequest) -> Option<String> {
    match req.headers().get("Authorization") {
        Some(value) => {
            let val = value.to_str().ok()?;
            for (dev, token) in DEVELOPER_TOKENS {
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
    );
}

#[derive(Deserialize)]
struct CreateProfileData {
    name: String,
}

async fn create_profile(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, info: web::Json<CreateProfileData>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = service.lock().unwrap();
    let profile = svc.create_profile("player", &info.name);
    HttpResponse::Ok().json(profile)
}

async fn get_profile(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let svc = service.lock().unwrap();
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

async fn set_dimensions(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>, info: web::Json<DimensionsData>) -> impl Responder {
    if authorized(&req).is_none() {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = service.lock().unwrap();
    let vec = BitVec { dim: info.dim, lanes: info.lanes.clone() };
    svc.set_vector(&path, vec);
    HttpResponse::Ok().finish()
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

async fn add_concept_to_profile(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>, info: web::Json<AssignConceptData>) -> impl Responder {
    match authorized(&req) {
        Some(dev) if dev == info.developer => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let reg = ConceptRegistry::load("concept_registry.json").unwrap_or_default();
    let key = format!("{}:{}:{}", info.developer, info.game, info.concept);
    if let Some(vec) = reg.get(&key) {
        let mut svc = service.lock().unwrap();
        svc.merge_vector(&path, vec);
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().finish()
    }
}

