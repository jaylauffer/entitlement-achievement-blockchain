use crate::achievement_registry::{AchievementDefinition, AchievementRegistry};
use crate::concept_registry::ConceptRegistry;
use crate::entitlement_registry::{EntitlementDefinition, EntitlementRegistry};
use crate::hd::BitVec;
use crate::identity::{exchange_identity, player_id_from_session, IdentityError};
use crate::player_profile::profile_service::{
    AchievementClaim, AchievementClaimInput, AchievementClaimReviewAction, AwardRecord,
    PlayerProfileService,
};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use once_cell::sync::Lazy;
use serde::Deserialize;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::RwLock;
use std::{collections::HashMap, env, fs};

const SCOPE_MANAGE_CONCEPTS: &str = "manage:concepts";
const SCOPE_REGISTER_DEFINITIONS: &str = "register:definitions";
const SCOPE_AWARD_ACHIEVEMENTS: &str = "award:achievements";
const SCOPE_GRANT_ENTITLEMENTS: &str = "grant:entitlements";

#[derive(Clone, Debug)]
struct DeveloperTokenAuth {
    developer: String,
    token: String,
    scopes: HashSet<String>,
}

#[derive(Deserialize)]
struct DeveloperTokenEntry {
    developer: String,
    token: String,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Deserialize)]
struct DeveloperTokenFile {
    tokens: Vec<DeveloperTokenEntry>,
}

fn default_developer_scopes() -> HashSet<String> {
    [
        SCOPE_MANAGE_CONCEPTS,
        SCOPE_REGISTER_DEFINITIONS,
        SCOPE_AWARD_ACHIEVEMENTS,
        SCOPE_GRANT_ENTITLEMENTS,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn normalize_scopes(scopes: Vec<String>) -> HashSet<String> {
    if scopes.is_empty() {
        return default_developer_scopes();
    }
    scopes
        .into_iter()
        .map(|scope| scope.trim().to_string())
        .filter(|scope| !scope.is_empty())
        .collect()
}

fn parse_developer_tokens_file(contents: &str) -> Option<Vec<DeveloperTokenAuth>> {
    if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(contents) {
        return Some(
            map.into_iter()
                .map(|(developer, token)| DeveloperTokenAuth {
                    developer,
                    token,
                    scopes: default_developer_scopes(),
                })
                .collect(),
        );
    }
    if let Ok(file) = serde_json::from_str::<DeveloperTokenFile>(contents) {
        return Some(
            file.tokens
                .into_iter()
                .map(|entry| DeveloperTokenAuth {
                    developer: entry.developer,
                    token: entry.token,
                    scopes: normalize_scopes(entry.scopes),
                })
                .collect(),
        );
    }
    if let Ok(entries) = serde_json::from_str::<Vec<DeveloperTokenEntry>>(contents) {
        return Some(
            entries
                .into_iter()
                .map(|entry| DeveloperTokenAuth {
                    developer: entry.developer,
                    token: entry.token,
                    scopes: normalize_scopes(entry.scopes),
                })
                .collect(),
        );
    }
    None
}

fn parse_developer_tokens_env(var: &str) -> Vec<DeveloperTokenAuth> {
    var.split(',')
        .filter_map(|entry| {
            let mut parts = entry.splitn(3, ':');
            let developer = parts.next()?.trim();
            let token = parts.next()?.trim();
            let scopes = parts
                .next()
                .map(|raw| {
                    raw.split('+')
                        .map(|scope| scope.trim().to_string())
                        .filter(|scope| !scope.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(DeveloperTokenAuth {
                developer: developer.to_string(),
                token: token.to_string(),
                scopes: normalize_scopes(scopes),
            })
        })
        .collect()
}

static DEVELOPER_TOKENS: Lazy<Vec<DeveloperTokenAuth>> = Lazy::new(|| {
    if let Ok(path) = env::var("DEVELOPER_TOKENS_FILE") {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Some(tokens) = parse_developer_tokens_file(&contents) {
                return tokens;
            }
        }
    }
    if let Ok(var) = env::var("DEVELOPER_TOKENS") {
        return parse_developer_tokens_env(&var);
    }
    vec![
        DeveloperTokenAuth {
            developer: "dev1".to_string(),
            token: "token1".to_string(),
            scopes: default_developer_scopes(),
        },
        DeveloperTokenAuth {
            developer: "dev2".to_string(),
            token: "token2".to_string(),
            scopes: default_developer_scopes(),
        },
    ]
});

const DEFAULT_CONCEPT_REGISTRY_PATH: &str = "concept_registry.json";
const DEFAULT_ACHIEVEMENT_REGISTRY_PATH: &str = "achievement_registry.json";
const DEFAULT_ENTITLEMENT_REGISTRY_PATH: &str = "entitlement_registry.json";

#[cfg(test)]
thread_local! {
    static TEST_REGISTRY_PATHS: RefCell<Option<RegistryPaths>> = const { RefCell::new(None) };
    static TEST_DEVELOPER_TOKENS: RefCell<Option<Vec<DeveloperTokenAuth>>> = const { RefCell::new(None) };
}

#[cfg(test)]
#[derive(Clone)]
struct RegistryPaths {
    concept: String,
    achievement: String,
    entitlement: String,
}

#[cfg(test)]
fn test_registry_path(which: fn(&RegistryPaths) -> &String) -> Option<String> {
    TEST_REGISTRY_PATHS.with(|paths| paths.borrow().as_ref().map(|paths| which(paths).clone()))
}

#[cfg(test)]
fn test_developer_tokens() -> Option<Vec<DeveloperTokenAuth>> {
    TEST_DEVELOPER_TOKENS.with(|tokens| tokens.borrow().clone())
}

fn concept_registry_path() -> String {
    #[cfg(test)]
    if let Some(path) = test_registry_path(|paths| &paths.concept) {
        return path;
    }
    env::var("CONCEPT_REGISTRY_PATH").unwrap_or_else(|_| DEFAULT_CONCEPT_REGISTRY_PATH.to_string())
}

fn achievement_registry_path() -> String {
    #[cfg(test)]
    if let Some(path) = test_registry_path(|paths| &paths.achievement) {
        return path;
    }
    env::var("ACHIEVEMENT_REGISTRY_PATH")
        .unwrap_or_else(|_| DEFAULT_ACHIEVEMENT_REGISTRY_PATH.to_string())
}

fn entitlement_registry_path() -> String {
    #[cfg(test)]
    if let Some(path) = test_registry_path(|paths| &paths.entitlement) {
        return path;
    }
    env::var("ENTITLEMENT_REGISTRY_PATH")
        .unwrap_or_else(|_| DEFAULT_ENTITLEMENT_REGISTRY_PATH.to_string())
}

fn authorized_token(req: &HttpRequest) -> Option<DeveloperTokenAuth> {
    match req.headers().get("Authorization") {
        Some(value) => {
            let val = value.to_str().ok()?;
            #[cfg(test)]
            if let Some(tokens) = test_developer_tokens() {
                for token in tokens {
                    if val == format!("Bearer {}", token.token) {
                        return Some(token);
                    }
                }
                return None;
            }
            for token in DEVELOPER_TOKENS.iter() {
                if val == format!("Bearer {}", token.token) {
                    return Some(token.clone());
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

fn developer_authorized_for(req: &HttpRequest, developer: &str, required_scope: &str) -> bool {
    matches!(
        authorized_token(req),
        Some(token) if token.developer == developer && token.scopes.contains(required_scope)
    )
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
        .service(
            web::resource("/profiles/{id}/achievement-claims")
                .route(web::get().to(list_achievement_claims_for_profile))
                .route(web::post().to(submit_achievement_claim_to_profile)),
        )
        .service(
            web::resource("/profiles/{id}/achievement-claims/{claim_id}/review")
                .route(web::post().to(review_achievement_claim_for_profile)),
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
    match authorized_token(&req) {
        Some(token)
            if token.developer == info.developer
                && token.scopes.contains(SCOPE_MANAGE_CONCEPTS) => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }
    let registry_path = concept_registry_path();
    let mut reg = ConceptRegistry::load(&registry_path).unwrap_or_default();
    let key = format!("{}:{}:{}", info.developer, info.game, info.concept);
    let dim = info
        .dim
        .unwrap_or(crate::player_profile::profile_service::DEFAULT_DIM);
    let vec = match reg.get(&key) {
        Some(v) => v.clone(),
        None => {
            let v = BitVec::seed(&key, dim);
            reg.insert(key.clone(), v.clone());
            let _ = reg.save(&registry_path);
            v
        }
    };
    HttpResponse::Ok().json(vec)
}

async fn get_concept(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
) -> impl Responder {
    let authorized = authorized_token(&req);
    if !matches!(
        authorized,
        Some(token)
            if token.developer == path.0 && token.scopes.contains(SCOPE_MANAGE_CONCEPTS)
    ) {
        return HttpResponse::Unauthorized().finish();
    }
    let registry_path = concept_registry_path();
    let reg = ConceptRegistry::load(&registry_path).unwrap_or_default();
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
    let registry_path = concept_registry_path();
    let reg = ConceptRegistry::load(&registry_path).unwrap_or_default();
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

#[derive(Deserialize)]
struct AchievementClaimData {
    developer: String,
    game: String,
    achievement_id: String,
    version: u32,
    claim_id: String,
    session_id: String,
    client_sequence: u64,
    claimed_at: String,
    evidence: Option<String>,
}

async fn submit_achievement_claim_to_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<String>,
    info: web::Json<AchievementClaimData>,
) -> impl Responder {
    let player_id = match player_id_from_request(&req) {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if player_id != path.as_str() {
        return HttpResponse::Unauthorized().finish();
    }

    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    let claim = AchievementClaimInput {
        developer: info.developer.clone(),
        game: info.game.clone(),
        achievement_id: info.achievement_id.clone(),
        version: info.version,
        claim_id: info.claim_id.clone(),
        session_id: info.session_id.clone(),
        client_sequence: info.client_sequence,
        claimed_at: info.claimed_at.clone(),
        evidence: info.evidence.clone(),
    };
    match svc.submit_achievement_claim(&path, claim) {
        Ok(stored) => HttpResponse::Accepted().json(stored),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

async fn list_achievement_claims_for_profile(
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
    let claims = svc
        .get_achievement_claims(&path)
        .cloned()
        .unwrap_or_default();
    HttpResponse::Ok().json(claims)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AchievementClaimReviewActionData {
    Promote,
    Reject,
}

#[derive(Deserialize)]
struct AchievementClaimReviewData {
    action: AchievementClaimReviewActionData,
    review_note: Option<String>,
}

#[derive(serde::Serialize)]
struct AchievementClaimReviewResponse {
    claim: AchievementClaim,
    #[serde(skip_serializing_if = "Option::is_none")]
    award: Option<AwardRecord>,
}

async fn review_achievement_claim_for_profile(
    service: web::Data<RwLock<PlayerProfileService>>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
    info: web::Json<AchievementClaimReviewData>,
) -> impl Responder {
    let Some(token) = authorized_token(&req) else {
        return HttpResponse::Unauthorized().finish();
    };
    if !token.scopes.contains(SCOPE_AWARD_ACHIEVEMENTS) {
        return HttpResponse::Unauthorized().finish();
    }

    let (player_id, claim_id) = path.into_inner();
    let claim_snapshot = {
        let svc = match service.read() {
            Ok(guard) => guard,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
        let Some(claim) = svc.get_achievement_claim(&player_id, &claim_id).cloned() else {
            return HttpResponse::NotFound().finish();
        };
        claim
    };
    if token.developer != claim_snapshot.developer {
        return HttpResponse::Unauthorized().finish();
    }

    let action = match info.action {
        AchievementClaimReviewActionData::Promote => AchievementClaimReviewAction::Promote,
        AchievementClaimReviewActionData::Reject => AchievementClaimReviewAction::Reject,
    };
    let achievement = match action {
        AchievementClaimReviewAction::Promote => {
            let reg = AchievementRegistry::load(&achievement_registry_path()).unwrap_or_default();
            let Some(def) = reg
                .get(
                    &claim_snapshot.developer,
                    &claim_snapshot.game,
                    &claim_snapshot.achievement_id,
                    claim_snapshot.version,
                )
                .cloned()
            else {
                return HttpResponse::NotFound().finish();
            };
            Some(def)
        }
        AchievementClaimReviewAction::Reject => None,
    };

    let mut svc = match service.write() {
        Ok(guard) => guard,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    match svc.review_achievement_claim(
        &player_id,
        &claim_id,
        &token.developer,
        action,
        info.review_note.clone(),
        achievement.as_ref(),
    ) {
        Ok((claim, award)) => HttpResponse::Ok().json(AchievementClaimReviewResponse { claim, award }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HttpResponse::NotFound().finish(),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
            HttpResponse::BadRequest().body(e.to_string())
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

async fn add_achievement(req: HttpRequest, info: web::Json<AchievementDefData>) -> impl Responder {
    if !developer_authorized_for(&req, &info.developer, SCOPE_REGISTER_DEFINITIONS) {
        return HttpResponse::Unauthorized().finish();
    }
    let registry_path = achievement_registry_path();
    let mut reg = AchievementRegistry::load(&registry_path).unwrap_or_default();
    let def = AchievementDefinition {
        developer: info.developer.clone(),
        game: info.game.clone(),
        achievement_id: info.achievement_id.clone(),
        version: info.version,
        name: info.name.clone(),
        description: info.description.clone(),
    };
    reg.insert(def);
    let _ = reg.save(&registry_path);
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
    if !developer_authorized_for(&req, &info.developer, SCOPE_AWARD_ACHIEVEMENTS) {
        return HttpResponse::Unauthorized().finish();
    }
    let registry_path = achievement_registry_path();
    let reg = AchievementRegistry::load(&registry_path).unwrap_or_default();
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
            Ok(receipt) => HttpResponse::Ok().json(receipt),
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
    if !developer_authorized_for(&req, &info.developer, SCOPE_REGISTER_DEFINITIONS) {
        return HttpResponse::Unauthorized().finish();
    }
    let registry_path = entitlement_registry_path();
    let mut reg = EntitlementRegistry::load(&registry_path).unwrap_or_default();
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
    let _ = reg.save(&registry_path);
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
    if !developer_authorized_for(&req, &info.developer, SCOPE_GRANT_ENTITLEMENTS) {
        return HttpResponse::Unauthorized().finish();
    }
    let registry_path = entitlement_registry_path();
    let reg = EntitlementRegistry::load(&registry_path).unwrap_or_default();
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
            Ok(receipt) => HttpResponse::Ok().json(receipt),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::issue_test_session;
    use crate::ledger_storage::FileTopicLedgerStorage;
    use actix_web::{http::StatusCode, test, App};
    use std::sync::Mutex;
    use uuid::Uuid;

    static API_TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct TestRegistryGuard {
        root: std::path::PathBuf,
    }

    struct TestDeveloperTokensGuard;

    impl TestRegistryGuard {
        fn enter(root: &std::path::Path) -> Self {
            TEST_REGISTRY_PATHS.with(|paths| {
                *paths.borrow_mut() = Some(RegistryPaths {
                    concept: root.join("concept_registry.json").display().to_string(),
                    achievement: root.join("achievement_registry.json").display().to_string(),
                    entitlement: root.join("entitlement_registry.json").display().to_string(),
                });
            });
            Self {
                root: root.to_path_buf(),
            }
        }
    }

    impl TestDeveloperTokensGuard {
        fn enter(tokens: Vec<DeveloperTokenAuth>) -> Self {
            TEST_DEVELOPER_TOKENS.with(|slot| {
                *slot.borrow_mut() = Some(tokens);
            });
            Self
        }
    }

    impl Drop for TestRegistryGuard {
        fn drop(&mut self) {
            TEST_REGISTRY_PATHS.with(|paths| {
                *paths.borrow_mut() = None;
            });
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    impl Drop for TestDeveloperTokensGuard {
        fn drop(&mut self) {
            TEST_DEVELOPER_TOKENS.with(|slot| {
                *slot.borrow_mut() = None;
            });
        }
    }

    fn test_service(root: &std::path::Path) -> web::Data<RwLock<PlayerProfileService>> {
        let storage = FileTopicLedgerStorage::new(root.join("player_logs"));
        web::Data::new(RwLock::new(PlayerProfileService::new(Box::new(storage))))
    }

    fn seed_profile(
        service: &web::Data<RwLock<PlayerProfileService>>,
        player_id: &str,
        name: &str,
    ) {
        service
            .write()
            .expect("service lock")
            .create_profile(player_id, name)
            .expect("create profile");
    }

    fn seed_achievement_definition() {
        let mut reg = AchievementRegistry::default();
        reg.insert(AchievementDefinition {
            developer: "dev1".into(),
            game: "game".into(),
            achievement_id: "first-win".into(),
            version: 1,
            name: "First Win".into(),
            description: "Win your first match".into(),
        });
        reg.save(&achievement_registry_path())
            .expect("save achievement registry");
    }

    fn seed_entitlement_definition() {
        let mut reg = EntitlementRegistry::default();
        reg.insert(EntitlementDefinition {
            developer: "dev1".into(),
            game: "game".into(),
            entitlement_id: "starter-pack".into(),
            version: 1,
            item_type: "item".into(),
            item_id: "starter-pack".into(),
            description: "Starter pack".into(),
        });
        reg.save(&entitlement_registry_path())
            .expect("save entitlement registry");
    }

    fn scoped_token(developer: &str, token: &str, scopes: &[&str]) -> DeveloperTokenAuth {
        DeveloperTokenAuth {
            developer: developer.to_string(),
            token: token.to_string(),
            scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
        }
    }

    #[actix_web::test]
    async fn player_session_cannot_self_award_achievement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{}/achievements", player_id))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn player_session_cannot_self_grant_entitlement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");
        seed_entitlement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{}/entitlements", player_id))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "entitlement_id": "starter-pack",
                "version": 1,
                "quantity": 1,
                "expiration_date": null
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn trusted_service_token_can_award_achievement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/achievements"))
            .insert_header(("Authorization", "Bearer token1"))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn mismatched_developer_token_cannot_award_achievement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/achievements"))
            .insert_header(("Authorization", "Bearer token2"))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn trusted_service_token_can_grant_entitlement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_entitlement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/entitlements"))
            .insert_header(("Authorization", "Bearer token1"))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "entitlement_id": "starter-pack",
                "version": 1,
                "quantity": 1,
                "expiration_date": null
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn player_session_can_submit_achievement_claim_for_own_profile() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/achievement-claims"))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1,
                "claim_id": "claim-1",
                "session_id": "offline-session-1",
                "client_sequence": 3,
                "claimed_at": "2026-03-22T09:00:00Z",
                "evidence": "won_match_offline"
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let service = service.read().expect("service lock");
        let claims = service
            .get_achievement_claims(&player_id)
            .expect("missing claims");
        assert_eq!(claims.len(), 1);
        let rewards = service.get_reward_state(&player_id).expect("missing rewards");
        assert!(rewards.achievements.is_empty());
    }

    #[actix_web::test]
    async fn player_session_cannot_submit_achievement_claim_for_other_profile() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let other_player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");
        seed_profile(&service, &other_player_id, "Player Two");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{other_player_id}/achievement-claims"))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1,
                "claim_id": "claim-2",
                "session_id": "offline-session-1",
                "client_sequence": 4,
                "claimed_at": "2026-03-22T09:01:00Z",
                "evidence": null
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn duplicate_achievement_claim_id_is_idempotent() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let make_request = || {
            test::TestRequest::post()
                .uri(&format!("/profiles/{player_id}/achievement-claims"))
                .insert_header(("Authorization", format!("Bearer {}", session_token)))
                .set_json(serde_json::json!({
                    "developer": "dev1",
                    "game": "game",
                    "achievement_id": "first-win",
                    "version": 1,
                    "claim_id": "claim-dup",
                    "session_id": "offline-session-2",
                    "client_sequence": 5,
                    "claimed_at": "2026-03-22T09:02:00Z",
                    "evidence": "duplicate"
                }))
                .to_request()
        };

        let first = test::call_service(&app, make_request()).await;
        let second = test::call_service(&app, make_request()).await;
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        assert_eq!(second.status(), StatusCode::ACCEPTED);

        let service = service.read().expect("service lock");
        let claims = service
            .get_achievement_claims(&player_id)
            .expect("missing claims");
        assert_eq!(claims.len(), 1);
        let rewards = service.get_reward_state(&player_id).expect("missing rewards");
        assert!(rewards.achievements.is_empty());
    }

    #[actix_web::test]
    async fn same_claim_id_is_allowed_for_different_players() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_one = Uuid::new_v4().to_string();
        let player_two = Uuid::new_v4().to_string();
        let session_one = issue_test_session(&player_one);
        let session_two = issue_test_session(&player_two);
        seed_profile(&service, &player_one, "Player One");
        seed_profile(&service, &player_two, "Player Two");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let make_request = |player_id: &str, session_token: &str| {
            test::TestRequest::post()
                .uri(&format!("/profiles/{player_id}/achievement-claims"))
                .insert_header(("Authorization", format!("Bearer {}", session_token)))
                .set_json(serde_json::json!({
                    "developer": "dev1",
                    "game": "game",
                    "achievement_id": "first-win",
                    "version": 1,
                    "claim_id": "shared-claim-id",
                    "session_id": "offline-session",
                    "client_sequence": 1,
                    "claimed_at": "2026-03-22T09:03:00Z",
                    "evidence": null
                }))
                .to_request()
        };

        let first = test::call_service(&app, make_request(&player_one, &session_one)).await;
        let second = test::call_service(&app, make_request(&player_two, &session_two)).await;
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        assert_eq!(second.status(), StatusCode::ACCEPTED);

        let service = service.read().expect("service lock");
        assert_eq!(
            service
                .get_achievement_claims(&player_one)
                .expect("claims one")
                .len(),
            1
        );
        assert_eq!(
            service
                .get_achievement_claims(&player_two)
                .expect("claims two")
                .len(),
            1
        );
    }

    #[actix_web::test]
    async fn achievement_claim_requires_existing_profile() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/achievement-claims"))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1,
                "claim_id": "claim-missing-profile",
                "session_id": "offline-session-missing",
                "client_sequence": 1,
                "claimed_at": "2026-03-22T09:04:00Z",
                "evidence": null
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn player_session_can_list_own_achievement_claims() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");
        service
            .write()
            .expect("service lock")
            .submit_achievement_claim(
                &player_id,
                AchievementClaimInput {
                    developer: "dev1".into(),
                    game: "game".into(),
                    achievement_id: "first-win".into(),
                    version: 1,
                    claim_id: "claim-list".into(),
                    session_id: "offline-session-list".into(),
                    client_sequence: 9,
                    claimed_at: "2026-03-22T09:05:00Z".into(),
                    evidence: Some("offline".into()),
                },
            )
            .expect("submit claim");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri(&format!("/profiles/{player_id}/achievement-claims"))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let claims: Vec<AchievementClaim> = test::read_body_json(resp).await;
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_id, "claim-list");
    }

    #[actix_web::test]
    async fn trusted_service_token_can_promote_pending_claim() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let _tokens = TestDeveloperTokensGuard::enter(vec![scoped_token(
            "dev1",
            "award-only",
            &[SCOPE_AWARD_ACHIEVEMENTS],
        )]);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();
        service
            .write()
            .expect("service lock")
            .submit_achievement_claim(
                &player_id,
                AchievementClaimInput {
                    developer: "dev1".into(),
                    game: "game".into(),
                    achievement_id: "first-win".into(),
                    version: 1,
                    claim_id: "claim-promote".into(),
                    session_id: "offline-session-promote".into(),
                    client_sequence: 2,
                    claimed_at: "2026-03-22T09:06:00Z".into(),
                    evidence: Some("offline".into()),
                },
            )
            .expect("submit claim");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!(
                "/profiles/{player_id}/achievement-claims/{}/review",
                "claim-promote"
            ))
            .insert_header(("Authorization", "Bearer award-only"))
            .set_json(serde_json::json!({
                "action": "promote",
                "review_note": "validated by consortium"
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["claim"]["status"], "Promoted");
        assert_eq!(body["claim"]["reviewer"], "dev1");
        assert_eq!(body["award"]["transaction_type"], "achievement");

        let service = service.read().expect("service lock");
        let rewards = service.get_reward_state(&player_id).expect("rewards");
        assert_eq!(rewards.achievements.len(), 1);
        let claims = service.get_achievement_claims(&player_id).expect("claims");
        assert_eq!(claims.len(), 1);
        assert!(claims[0].awarded_transaction_id.is_some());
    }

    #[actix_web::test]
    async fn player_session_cannot_review_claim() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        let session_token = issue_test_session(&player_id);
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();
        service
            .write()
            .expect("service lock")
            .submit_achievement_claim(
                &player_id,
                AchievementClaimInput {
                    developer: "dev1".into(),
                    game: "game".into(),
                    achievement_id: "first-win".into(),
                    version: 1,
                    claim_id: "claim-no-review".into(),
                    session_id: "offline-session-review".into(),
                    client_sequence: 10,
                    claimed_at: "2026-03-22T09:07:00Z".into(),
                    evidence: None,
                },
            )
            .expect("submit claim");

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!(
                "/profiles/{player_id}/achievement-claims/{}/review",
                "claim-no-review"
            ))
            .insert_header(("Authorization", format!("Bearer {}", session_token)))
            .set_json(serde_json::json!({
                "action": "promote",
                "review_note": "player tried to self-award"
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn token_without_award_scope_cannot_award_achievement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let _tokens = TestDeveloperTokensGuard::enter(vec![scoped_token(
            "dev1",
            "defs-only",
            &[SCOPE_REGISTER_DEFINITIONS],
        )]);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_achievement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/achievements"))
            .insert_header(("Authorization", "Bearer defs-only"))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "achievement_id": "first-win",
                "version": 1
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn token_without_grant_scope_cannot_grant_entitlement() {
        let _lock = API_TEST_MUTEX.lock().expect("api test lock");
        let dir = std::env::temp_dir().join(format!("eab_api_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let _guard = TestRegistryGuard::enter(&dir);
        let _tokens = TestDeveloperTokensGuard::enter(vec![scoped_token(
            "dev1",
            "award-only",
            &[SCOPE_AWARD_ACHIEVEMENTS],
        )]);
        let service = test_service(&dir);
        let player_id = Uuid::new_v4().to_string();
        seed_profile(&service, &player_id, "Player One");
        seed_entitlement_definition();

        let app = test::init_service(
            App::new()
                .app_data(service.clone())
                .configure(init_routes),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&format!("/profiles/{player_id}/entitlements"))
            .insert_header(("Authorization", "Bearer award-only"))
            .set_json(serde_json::json!({
                "developer": "dev1",
                "game": "game",
                "entitlement_id": "starter-pack",
                "version": 1,
                "quantity": 1,
                "expiration_date": null
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
