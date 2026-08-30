use crate::player_profile::profile_service::PlayerProfile;
use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Version of the application recorded in each block
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entitlement {
    pub developer: String,
    pub game: String,
    pub entitlement_id: String,
    pub version: u32,
    pub item_type: String,
    pub item_id: String,
    pub quantity: u32,
    pub metadata: String,
    pub expiration_date: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Achievement {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub achievement_name: String,
    pub criteria: String,
    pub timestamp_earned: String,
    pub metadata: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TransactionData {
    Entitlement(Entitlement),
    Achievement(Achievement),
    ProfileChange(ProfileChange),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProfileChange {
    pub profile_hash: String,
    pub profile: PlayerProfile,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub transaction_id: String,
    pub player_id: String,
    pub transaction_type: String,
    pub timestamp: String,
    pub data_hash: String,
    pub signature: String,
    pub details: TransactionData,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Block {
    pub block_hash: String,
    pub previous_block_hash: String,
    pub timestamp: String,
    pub app_version: String,
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
}

#[derive(Debug)]
pub struct Blockchain {
    pub chain: Vec<Block>,
}

impl Default for Blockchain {
    fn default() -> Self {
        Self::new()
    }
}

impl Blockchain {
    pub fn new() -> Self {
        Blockchain {
            chain: vec![Blockchain::create_genesis_block()],
        }
    }

    fn create_genesis_block() -> Block {
        Block {
            block_hash: String::from("0"),
            previous_block_hash: String::from("0"),
            timestamp: Utc::now().to_rfc3339(),
            app_version: APP_VERSION.to_string(),
            nonce: 0,
            transactions: vec![],
        }
    }

    pub fn get_latest_block(&self) -> Option<&Block> {
        self.chain.last()
    }

    pub fn add_block(&mut self, transactions: Vec<Transaction>) {
        if let Some(previous_block) = self.get_latest_block() {
            let new_block = Blockchain::create_block(previous_block, transactions);
            self.chain.push(new_block);
        }
    }

    fn create_block(previous_block: &Block, transactions: Vec<Transaction>) -> Block {
        let mut nonce = 0;
        let timestamp = Utc::now().to_rfc3339();
        let mut block_hash = Blockchain::calculate_hash(
            &previous_block.block_hash,
            &transactions,
            &timestamp,
            APP_VERSION,
            nonce,
        );
        while !block_hash.starts_with("00") {
            nonce += 1;
            block_hash = Blockchain::calculate_hash(
                &previous_block.block_hash,
                &transactions,
                &timestamp,
                APP_VERSION,
                nonce,
            );
        }
        Block {
            block_hash,
            previous_block_hash: previous_block.block_hash.clone(),
            timestamp,
            app_version: APP_VERSION.to_string(),
            nonce,
            transactions,
        }
    }

    fn calculate_hash(
        previous_block_hash: &str,
        transactions: &Vec<Transaction>,
        timestamp: &str,
        app_version: &str,
        nonce: u64,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(previous_block_hash);
        let json = serde_json::to_string(transactions)
            .expect("failed to serialize transactions for hashing");
        hasher.update(&json);
        hasher.update(timestamp);
        hasher.update(app_version);
        hasher.update(nonce.to_string());
        let result = hasher.finalize();
        hex::encode(result)
    }

    pub fn is_valid_chain(&self) -> bool {
        for i in 1..self.chain.len() {
            let current_block = &self.chain[i];
            let previous_block = &self.chain[i - 1];
            if current_block.previous_block_hash != previous_block.block_hash {
                return false;
            }
            let recalculated_hash = Blockchain::calculate_hash(
                &current_block.previous_block_hash,
                &current_block.transactions,
                &current_block.timestamp,
                &current_block.app_version,
                current_block.nonce,
            );
            if current_block.block_hash != recalculated_hash {
                return false;
            }
        }
        true
    }
}
