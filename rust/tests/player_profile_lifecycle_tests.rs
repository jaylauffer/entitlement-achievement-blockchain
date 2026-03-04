use rust_blockchain::blockchain::TransactionData;
use rust_blockchain::hd::{hamming_distance, BitVec};
use rust_blockchain::ledger_storage::FileTopicLedgerStorage;
use rust_blockchain::player_profile::profile_service::{PlayerProfileService, DEFAULT_DIM};
use uuid::Uuid;

#[test]
fn test_end_to_end_profile_lifecycle_with_ledger_reload() {
    let dir = "test_player_lifecycle_logs";
    let storage = FileTopicLedgerStorage::new(dir);
    let mut service = PlayerProfileService::new(Box::new(storage));
    let pid = Uuid::new_v4().to_string();

    service
        .create_profile(&pid, "Lifecycle")
        .expect("create profile");
    let base_vec = BitVec::seed("BASE", DEFAULT_DIM);
    service
        .set_vector(&pid, base_vec.clone())
        .expect("set vector");

    let merge_vec = BitVec::seed("MERGE", DEFAULT_DIM);
    service
        .merge_vector(&pid, &merge_vec)
        .expect("merge vector");

    let ach = rust_blockchain::achievement_registry::AchievementDefinition {
        developer: "dev".into(),
        game: "game".into(),
        achievement_id: "ach1".into(),
        version: 1,
        name: "First".into(),
        description: "Earned".into(),
    };

    let ent = rust_blockchain::entitlement_registry::EntitlementDefinition {
        developer: "dev".into(),
        game: "game".into(),
        entitlement_id: "ent1".into(),
        version: 1,
        item_type: "item".into(),
        item_id: "i1".into(),
        description: "desc".into(),
    };

    service
        .award_achievement(&pid, &ach)
        .expect("award achievement");
    service
        .award_entitlement(&pid, &ent, 2, None)
        .expect("award entitlement");

    assert_eq!(service.ledger.chain.len(), 6);
    assert!(service.ledger.is_valid_chain());

    drop(service);

    let storage = FileTopicLedgerStorage::new(dir);
    let service = PlayerProfileService::new(Box::new(storage));
    assert_eq!(service.ledger.chain.len(), 6);

    let profile = service.get_profile(&pid).expect("missing profile");
    let expected_vec = base_vec.xor(&merge_vec);
    assert_eq!(hamming_distance(&profile.profile_vec, &expected_vec), 0);

    let mut has_achievement = false;
    let mut has_entitlement = false;
    for txn in service
        .ledger
        .chain
        .iter()
        .flat_map(|block| &block.transactions)
    {
        match txn.details {
            TransactionData::Achievement(_) => has_achievement = true,
            TransactionData::Entitlement(_) => has_entitlement = true,
            _ => {}
        }
    }

    assert!(has_achievement);
    assert!(has_entitlement);

    let _ = std::fs::remove_dir_all(dir);
}
