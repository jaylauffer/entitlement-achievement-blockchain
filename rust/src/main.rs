// main.rs

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use chrono::prelude::*;
use std::fmt;

use rust_blockchain::player_profile::profile_service::*;
use rust_blockchain::hd::{BitVec, hamming_distance};

// Define a struct to represent an entitlement
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Entitlement {
    entitlement_id: String,
    item_type: String,
    item_id: String,
    quantity: u32,
    metadata: String,
    expiration_date: Option<String>,
}

// Define a struct to represent an achievement
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Achievement {
    achievement_id: String,
    achievement_name: String,
    criteria: String,
    timestamp_earned: String,
    metadata: String,
}

// Define a struct to represent a transaction
#[derive(Serialize, Deserialize, Debug, Clone)]
enum TransactionData {
    Entitlement(Entitlement),
    Achievement(Achievement),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Transaction {
    transaction_id: String,
    player_id: String,
    transaction_type: String, // "entitlement" or "achievement"
    timestamp: String,
    data_hash: String,
    signature: String,
    details: TransactionData,
}

// Define a struct to represent a block in the blockchain
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Block {
    block_hash: String,
    previous_block_hash: String,
    timestamp: String,
    nonce: u64,
    transactions: Vec<Transaction>,
}

// Define the blockchain itself
#[derive(Debug)]
struct Blockchain {
    chain: Vec<Block>,
}

impl Blockchain {
    // Initialize a new blockchain with a genesis block
    fn new() -> Self {
        Blockchain {
            chain: vec![Blockchain::create_genesis_block()],
        }
    }

    // Create the genesis block
    fn create_genesis_block() -> Block {
        Block {
            block_hash: String::from("0"),
            previous_block_hash: String::from("0"),
            timestamp: Utc::now().to_rfc3339(),
            nonce: 0,
            transactions: vec![],
        }
    }

    // Get the latest block in the blockchain
    fn get_latest_block(&self) -> &Block {
        self.chain.last().unwrap()
    }

    // Add a new block to the blockchain
    fn add_block(&mut self, transactions: Vec<Transaction>) {
        let previous_block = self.get_latest_block();
        let new_block = Blockchain::create_block(previous_block, transactions);
        self.chain.push(new_block);
    }

    // Create a new block
    fn create_block(previous_block: &Block, transactions: Vec<Transaction>) -> Block {
        let mut nonce = 0;
        let timestamp = Utc::now().to_rfc3339();
        let mut block_hash = Blockchain::calculate_hash(
            &previous_block.block_hash,
            &transactions,
            &timestamp,
            nonce,
        );

        // Simple proof-of-work: find a hash with a prefix of "00"
        while !block_hash.starts_with("00") {
            nonce += 1;
            block_hash = Blockchain::calculate_hash(
                &previous_block.block_hash,
                &transactions,
                &timestamp,
                nonce,
            );
        }

        Block {
            block_hash,
            previous_block_hash: previous_block.block_hash.clone(),
            timestamp,
            nonce,
            transactions,
        }
    }

    // Calculate the hash of a block
    fn calculate_hash(
        previous_block_hash: &str,
        transactions: &Vec<Transaction>,
        timestamp: &str,
        nonce: u64,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(previous_block_hash);
        hasher.update(&serde_json::to_string(transactions).unwrap());
        hasher.update(timestamp);
        hasher.update(nonce.to_string());
        let result = hasher.finalize();
        hex::encode(result)
    }

    // Validate the integrity of the blockchain
    fn is_valid_chain(&self) -> bool {
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
                current_block.nonce,
            );

            if current_block.block_hash != recalculated_hash {
                return false;
            }
        }
        true
    }
}

fn main() {
    // Create a new blockchain
    let mut blockchain = Blockchain::new();

    // Create the player profile service and a profile
    let mut profile_service = PlayerProfileService::new();
    profile_service.create_profile("player123", "Hero");
    let init_vec = BitVec::seed("INIT", DEFAULT_DIM);
    profile_service.set_vector("player123", init_vec);

    // Create some example transactions
    let entitlement = Entitlement {
        entitlement_id: String::from("ent123"),
        item_type: String::from("weapon"),
        item_id: String::from("sword_001"),
        quantity: 1,
        metadata: String::from("{\"rarity\": \"legendary\", \"attack_power\": 150}"),
        expiration_date: None,
    };

    let achievement = Achievement {
        achievement_id: String::from("ach001"),
        achievement_name: String::from("Master Swordsman"),
        criteria: String::from("kill_100_enemies"),
        timestamp_earned: Utc::now().to_rfc3339(),
        metadata: String::from("{\"difficulty\": \"hard\", \"reward\": \"sword_001\"}"),
    };

    let transaction1 = Transaction {
        transaction_id: String::from("tx12345"),
        player_id: String::from("player123"),
        transaction_type: String::from("entitlement"),
        timestamp: Utc::now().to_rfc3339(),
        data_hash: String::from("abc123hash"),
        signature: String::from("player_signature"),
        details: TransactionData::Entitlement(entitlement),
    };

    let transaction2 = Transaction {
        transaction_id: String::from("tx67890"),
        player_id: String::from("player123"),
        transaction_type: String::from("achievement"),
        timestamp: Utc::now().to_rfc3339(),
        data_hash: String::from("def456hash"),
        signature: String::from("player_signature"),
        details: TransactionData::Achievement(achievement),
    };

    // Add a block containing these transactions
    blockchain.add_block(vec![transaction1, transaction2]);

    // Print the blockchain
    for block in &blockchain.chain {
        println!("{:#?}", block);
    }

    // Validate the blockchain
    println!("Is blockchain valid? {}", blockchain.is_valid_chain());

    // Display the player profile
    if let Some(profile) = profile_service.get_profile("player123") {
        println!("Player Profile: {:?}", profile);
    }
}
