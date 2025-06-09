use actix_web::{web, HttpResponse, Responder, HttpRequest};
use crate::player_profile::profile_service::PlayerProfileService;
use crate::hd::BitVec;
use serde::Deserialize;

const AUTH_TOKEN: &str = "secret";

fn authorized(req: &HttpRequest) -> bool {
    match req.headers().get("Authorization") {
        Some(value) => value.to_str().map(|v| v == format!("Bearer {}", AUTH_TOKEN)).unwrap_or(false),
        None => false,
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
    );
}

#[derive(Deserialize)]
struct CreateProfileData {
    name: String,
}

async fn create_profile(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, info: web::Json<CreateProfileData>) -> impl Responder {
    if !authorized(&req) {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = service.lock().unwrap();
    let profile = svc.create_profile("player", &info.name);
    HttpResponse::Ok().json(profile)
}

async fn get_profile(service: web::Data<std::sync::Mutex<PlayerProfileService>>, req: HttpRequest, path: web::Path<String>) -> impl Responder {
    if !authorized(&req) {
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
    if !authorized(&req) {
        return HttpResponse::Unauthorized().finish();
    }
    let mut svc = service.lock().unwrap();
    let vec = BitVec { dim: info.dim, lanes: info.lanes.clone() };
    svc.set_vector(&path, vec);
    HttpResponse::Ok().finish()
}

