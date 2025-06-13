use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::hd::BitVec;

#[derive(Serialize, Deserialize, Default)]
pub struct ConceptRegistry {
    concepts: HashMap<String, BitVec>,
}

impl ConceptRegistry {
    pub fn load(path: &str) -> std::io::Result<Self> {
        match File::open(path) {
            Ok(mut f) => {
                let mut data = String::new();
                f.read_to_string(&mut data)?;
                let registry: ConceptRegistry = serde_json::from_str(&data).unwrap_or_default();
                Ok(registry)
            }
            Err(_) => Ok(ConceptRegistry::default()),
        }
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn insert(&mut self, key: String, vec: BitVec) {
        self.concepts.insert(key, vec);
    }

    pub fn get(&self, key: &str) -> Option<&BitVec> {
        self.concepts.get(key)
    }
}
