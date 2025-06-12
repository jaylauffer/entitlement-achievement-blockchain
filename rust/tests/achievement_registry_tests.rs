use rust_blockchain::achievement_registry::{AchievementRegistry, AchievementDefinition};

#[test]
fn test_insert_and_get() {
    let mut reg = AchievementRegistry::default();
    let def = AchievementDefinition {
        developer: "dev".into(),
        game: "game".into(),
        achievement_id: "ach1".into(),
        version: 1,
        name: "First".into(),
        description: "Desc".into(),
    };
    reg.insert(def.clone());
    assert!(reg.get("dev", "game", "ach1", 1).is_some());
    assert_eq!(reg.get("dev", "game", "ach1", 1).unwrap().name, "First");
}

#[test]
fn test_save_and_load() {
    let path = "test_achievements.json";
    let mut reg = AchievementRegistry::default();
    let def = AchievementDefinition {
        developer: "dev".into(),
        game: "game".into(),
        achievement_id: "ach1".into(),
        version: 1,
        name: "First".into(),
        description: "Desc".into(),
    };
    reg.insert(def);
    reg.save(path).unwrap();
    let loaded = AchievementRegistry::load(path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(loaded.get("dev", "game", "ach1", 1).is_some());
}
