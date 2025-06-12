use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntitlementDefinition {
    pub developer: String,
    pub game: String,
    pub entitlement_id: String,
    pub version: u32,
    pub item_type: String,
    pub item_id: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct EntitlementRegistry {
    entitlements: HashMap<String, EntitlementDefinition>,
}

impl EntitlementRegistry {
    pub fn load(path: &str) -> std::io::Result<Self> {
        match File::open(path) {
            Ok(mut f) => {
                let mut data = String::new();
                f.read_to_string(&mut data)?;
                let reg: EntitlementRegistry = serde_json::from_str(&data).unwrap_or_default();
                Ok(reg)
            }
            Err(_) => Ok(EntitlementRegistry::default()),
        }
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap();
        let mut file = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn insert(&mut self, def: EntitlementDefinition) {
        let key = Self::key(&def.developer, &def.game, &def.entitlement_id, def.version);
        self.entitlements.insert(key, def);
    }

    pub fn get(&self, developer: &str, game: &str, id: &str, version: u32) -> Option<&EntitlementDefinition> {
        let key = Self::key(developer, game, id, version);
        self.entitlements.get(&key)
    }

    fn key(dev: &str, game: &str, id: &str, version: u32) -> String {
        format!("{}:{}:{}:v{}", dev, game, id, version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut reg = EntitlementRegistry::default();
        let def = EntitlementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            entitlement_id: "ent1".into(),
            version: 1,
            item_type: "type".into(),
            item_id: "item".into(),
            description: "desc".into(),
        };
        reg.insert(def.clone());
        assert!(reg.get("dev", "game", "ent1", 1).is_some());
        assert_eq!(reg.get("dev", "game", "ent1", 1).unwrap().item_id, "item");
    }
}
