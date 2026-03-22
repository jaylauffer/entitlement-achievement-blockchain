use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::RwLock;
use uuid::Uuid;

const DEFAULT_IDENTITY_MAP_PATH: &str = "identity_map.json";
const DEFAULT_SUPPORTED_PROVIDERS: [&str; 5] =
    ["google_play_games", "apple_id", "epic", "steam", "oidc"];

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct IdentityMap {
    providers: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct ProviderTokenMap {
    tokens: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct ExchangeResult {
    pub access_token: String,
    pub player_id: String,
    pub is_new_player: bool,
}

#[derive(Debug)]
pub enum IdentityError {
    UnsupportedProvider,
    InvalidToken,
    StorageError,
}

#[derive(Default)]
struct SessionStore {
    sessions: HashMap<String, String>,
}

static IDENTITY_MAP_PATH: Lazy<String> = Lazy::new(|| {
    env::var("IDENTITY_MAP_PATH").unwrap_or_else(|_| DEFAULT_IDENTITY_MAP_PATH.to_string())
});

static SUPPORTED_PROVIDERS: Lazy<Vec<String>> = Lazy::new(|| {
    if let Ok(var) = env::var("SUPPORTED_IDENTITY_PROVIDERS") {
        var.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        DEFAULT_SUPPORTED_PROVIDERS
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
});

static PROVIDER_TOKENS: Lazy<ProviderTokenMap> = Lazy::new(|| {
    if let Ok(path) = env::var("IDENTITY_PROVIDER_TOKENS_FILE") {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<ProviderTokenMap>(&contents) {
                return map;
            }
        }
    }
    if let Ok(var) = env::var("IDENTITY_PROVIDER_TOKENS") {
        let mut tokens: HashMap<String, HashMap<String, String>> = HashMap::new();
        for entry in var.split(',') {
            let mut parts = entry.splitn(3, ':');
            if let (Some(provider), Some(token), Some(subject)) =
                (parts.next(), parts.next(), parts.next())
            {
                tokens
                    .entry(provider.trim().to_string())
                    .or_default()
                    .insert(token.trim().to_string(), subject.trim().to_string());
            }
        }
        return ProviderTokenMap { tokens };
    }
    ProviderTokenMap::default()
});

static IDENTITY_MAP: Lazy<RwLock<IdentityMap>> = Lazy::new(|| RwLock::new(load_identity_map()));
static SESSION_STORE: Lazy<RwLock<SessionStore>> =
    Lazy::new(|| RwLock::new(SessionStore::default()));

fn load_identity_map() -> IdentityMap {
    if let Ok(contents) = fs::read_to_string(&*IDENTITY_MAP_PATH) {
        if let Ok(map) = serde_json::from_str::<IdentityMap>(&contents) {
            return map;
        }
    }
    IdentityMap::default()
}

fn persist_identity_map(map: &IdentityMap) -> Result<(), IdentityError> {
    let contents = serde_json::to_string_pretty(map).map_err(|_| IdentityError::StorageError)?;
    fs::write(&*IDENTITY_MAP_PATH, contents).map_err(|_| IdentityError::StorageError)?;
    Ok(())
}

fn resolve_subject(provider: &str, token: &str) -> Option<String> {
    if let Some(provider_tokens) = PROVIDER_TOKENS.tokens.get(provider) {
        provider_tokens.get(token).cloned()
    } else if PROVIDER_TOKENS.tokens.is_empty() {
        Some(token.to_string())
    } else {
        None
    }
}

fn is_supported_provider(provider: &str) -> bool {
    SUPPORTED_PROVIDERS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(provider))
}

pub fn exchange_identity(provider: &str, token: &str) -> Result<ExchangeResult, IdentityError> {
    if !is_supported_provider(provider) {
        return Err(IdentityError::UnsupportedProvider);
    }
    let subject = resolve_subject(provider, token).ok_or(IdentityError::InvalidToken)?;
    let mut map_guard = IDENTITY_MAP
        .write()
        .map_err(|_| IdentityError::StorageError)?;
    let provider_entry = map_guard.providers.entry(provider.to_string()).or_default();
    let (player_id, is_new_player) = match provider_entry.get(&subject) {
        Some(existing) => (existing.clone(), false),
        None => {
            let new_id = Uuid::new_v4().to_string();
            provider_entry.insert(subject.clone(), new_id.clone());
            (new_id, true)
        }
    };
    persist_identity_map(&map_guard)?;
    let access_token = Uuid::new_v4().to_string();
    let mut sessions = SESSION_STORE
        .write()
        .map_err(|_| IdentityError::StorageError)?;
    sessions
        .sessions
        .insert(access_token.clone(), player_id.clone());
    Ok(ExchangeResult {
        access_token,
        player_id,
        is_new_player,
    })
}

pub fn player_id_from_session(token: &str) -> Option<String> {
    let sessions = SESSION_STORE.read().ok()?;
    sessions.sessions.get(token).cloned()
}

#[cfg(test)]
pub fn issue_test_session(player_id: &str) -> String {
    let access_token = Uuid::new_v4().to_string();
    let mut sessions = SESSION_STORE
        .write()
        .expect("test session store lock should not be poisoned");
    sessions
        .sessions
        .insert(access_token.clone(), player_id.to_string());
    access_token
}
