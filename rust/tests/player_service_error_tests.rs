use loadngo_eab::hd::BitVec;
use loadngo_eab::ledger_storage::FileTopicLedgerStorage;
use loadngo_eab::player_profile::profile_service::{PlayerProfileService, DEFAULT_DIM};
use uuid::Uuid;

#[test]
fn test_set_vector_missing_profile() {
    let dir = "test_err_logs";
    let storage = FileTopicLedgerStorage::new(dir);
    let mut service = PlayerProfileService::new(Box::new(storage));
    let pid = Uuid::new_v4().to_string();
    let vec = BitVec::seed("TEST", DEFAULT_DIM);
    let res = service.set_vector(&pid, vec);
    assert!(res.is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_award_achievement_missing_profile() {
    let dir = "test_err_logs2";
    let storage = FileTopicLedgerStorage::new(dir);
    let mut service = PlayerProfileService::new(Box::new(storage));
    let pid = Uuid::new_v4().to_string();
    let def = loadngo_eab::achievement_registry::AchievementDefinition {
        developer: "d".into(),
        game: "g".into(),
        achievement_id: "a".into(),
        version: 1,
        name: "n".into(),
        description: "desc".into(),
    };
    let res = service.award_achievement(&pid, &def);
    assert!(res.is_err());
    let _ = std::fs::remove_dir_all(dir);
}
