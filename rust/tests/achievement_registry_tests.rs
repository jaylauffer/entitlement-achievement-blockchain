use loadngo_eab::achievement_registry::{
    AchievementAccomplishment, AchievementDefinition, AchievementIssuanceMode, AchievementRegistry,
    AchievementRepeatability, AchievementVisibility,
};

#[test]
fn test_insert_and_get() {
    let mut reg = AchievementRegistry::default();
    let def = AchievementDefinition::new("dev", "game", "ach1", 1, "First", "Desc")
        .with_category("progression")
        .with_policy(
            AchievementVisibility::PublicProof,
            AchievementRepeatability::OncePerPlayer,
            AchievementIssuanceMode::DirectAwardOrClaimReview,
        )
        .with_accomplishment(AchievementAccomplishment {
            summary: "Do the first thing".into(),
            event_key: Some("first_thing".into()),
            threshold: Some(1),
            requires_evidence: false,
        });
    reg.insert(def.clone());
    assert!(reg.get("dev", "game", "ach1", 1).is_some());
    assert_eq!(
        reg.get("dev", "game", "ach1", 1).expect("ach").name(),
        "First"
    );
}

#[test]
fn test_save_and_load() {
    let path = "test_achievements.json";
    let mut reg = AchievementRegistry::default();
    let def = AchievementDefinition::new("dev", "game", "ach1", 1, "First", "Desc")
        .with_category("progression")
        .with_policy(
            AchievementVisibility::PublicProof,
            AchievementRepeatability::OncePerPlayer,
            AchievementIssuanceMode::DirectAwardOrClaimReview,
        )
        .with_accomplishment(AchievementAccomplishment {
            summary: "Do the first thing".into(),
            event_key: Some("first_thing".into()),
            threshold: Some(1),
            requires_evidence: false,
        });
    reg.insert(def);
    reg.save(path).expect("save");
    let loaded = AchievementRegistry::load(path).expect("load");
    let _ = std::fs::remove_file(path);
    assert!(loaded.get("dev", "game", "ach1", 1).is_some());
    let def = loaded.get("dev", "game", "ach1", 1).expect("loaded def");
    assert_eq!(def.policy.visibility, AchievementVisibility::PublicProof);
    assert_eq!(def.accomplishment.summary, "Do the first thing");
}

#[test]
fn test_current_definition_defaults_policy_fields_when_omitted() {
    let path = "test_achievement_defaults.json";
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
    .expect("write registry");
    let loaded = AchievementRegistry::load(path).expect("load");
    let _ = std::fs::remove_file(path);
    let def = loaded.get("dev", "game", "ach1", 1).expect("definition");
    assert_eq!(def.category(), "");
    assert_eq!(def.policy.visibility, AchievementVisibility::Private);
    assert_eq!(
        def.policy.repeatability,
        AchievementRepeatability::OncePerPlayer
    );
    assert_eq!(
        def.policy.issuance_mode,
        AchievementIssuanceMode::DirectAwardOrClaimReview
    );
    assert_eq!(def.accomplishment.summary, "");
    assert_eq!(def.accomplishment_summary(), "Desc");
}
