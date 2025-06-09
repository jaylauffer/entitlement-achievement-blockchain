use rust_blockchain::concept_registry::ConceptRegistry;
use rust_blockchain::hd::BitVec;
use std::fs;

#[test]
fn test_insert_and_get() {
    let mut reg = ConceptRegistry::default();
    let vec = BitVec::seed("game:concept", 128);
    reg.insert("game:concept".to_string(), vec.clone());
    assert!(reg.get("game:concept").is_some());
    assert_eq!(reg.get("game:concept").unwrap().lanes, vec.lanes);
}

#[test]
fn test_save_and_load() {
    let path = "test_registry.json";
    let mut reg = ConceptRegistry::default();
    let vec = BitVec::seed("game:test", 64);
    reg.insert("game:test".to_string(), vec.clone());
    reg.save(path).unwrap();
    let loaded = ConceptRegistry::load(path).unwrap();
    fs::remove_file(path).unwrap();
    assert!(loaded.get("game:test").is_some());
    assert_eq!(loaded.get("game:test").unwrap().lanes, vec.lanes);
}
