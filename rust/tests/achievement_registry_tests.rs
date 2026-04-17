use loadngo_eab::achievement_registry::{
    AchievementDefinition, AchievementIssuanceMode, AchievementRegistry, AchievementRepeatability,
    AchievementSuccessCriteria, AchievementVisibility,
};

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
        category: "progression".into(),
        visibility: AchievementVisibility::PublicProof,
        repeatability: AchievementRepeatability::OncePerPlayer,
        issuance_mode: AchievementIssuanceMode::DirectAwardOrClaimReview,
        success_criteria: AchievementSuccessCriteria {
            summary: "Do the first thing".into(),
            event_key: Some("first_thing".into()),
            threshold: Some(1),
            requires_evidence: false,
        },
    };
    reg.insert(def.clone());
    assert!(reg.get("dev", "game", "ach1", 1).is_some());
    assert_eq!(
        reg.get("dev", "game", "ach1", 1).expect("ach").name,
        "First"
    );
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
        category: "progression".into(),
        visibility: AchievementVisibility::PublicProof,
        repeatability: AchievementRepeatability::OncePerPlayer,
        issuance_mode: AchievementIssuanceMode::DirectAwardOrClaimReview,
        success_criteria: AchievementSuccessCriteria {
            summary: "Do the first thing".into(),
            event_key: Some("first_thing".into()),
            threshold: Some(1),
            requires_evidence: false,
        },
    };
    reg.insert(def);
    reg.save(path).expect("save");
    let loaded = AchievementRegistry::load(path).expect("load");
    let _ = std::fs::remove_file(path);
    assert!(loaded.get("dev", "game", "ach1", 1).is_some());
    let def = loaded.get("dev", "game", "ach1", 1).expect("loaded def");
    assert_eq!(def.visibility, AchievementVisibility::PublicProof);
    assert_eq!(def.success_criteria.summary, "Do the first thing");
}

#[test]
fn test_load_legacy_definition_defaults_policy_fields() {
    let path = "test_achievement_legacy.json";
    std::fs::write(
        path,
        r#"{
  "achievements": {
    "dev:game:ach1:v1": {
      "developer": "dev",
      "game": "game",
      "achievement_id": "ach1",
      "version": 1,
      "name": "First",
      "description": "Desc"
    }
  }
}"#,
    )
    .expect("write legacy registry");
    let loaded = AchievementRegistry::load(path).expect("load");
    let _ = std::fs::remove_file(path);
    let def = loaded.get("dev", "game", "ach1", 1).expect("legacy def");
    assert_eq!(def.category, "");
    assert_eq!(def.visibility, AchievementVisibility::Private);
    assert_eq!(def.repeatability, AchievementRepeatability::OncePerPlayer);
    assert_eq!(
        def.issuance_mode,
        AchievementIssuanceMode::DirectAwardOrClaimReview
    );
    assert_eq!(def.success_criteria.summary, "");
    assert_eq!(def.criteria_summary(), "Desc");
}
