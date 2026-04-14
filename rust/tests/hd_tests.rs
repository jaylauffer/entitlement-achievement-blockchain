use loadngo_eab::hd::{hamming_distance, threshold_sum, BitVec};

#[test]
fn test_seed_consistency() {
    let v1 = BitVec::seed("SAME", 128);
    let v2 = BitVec::seed("SAME", 128);
    assert_eq!(v1.lanes, v2.lanes);
}

#[test]
fn test_hamming_distance_zero() {
    let v = BitVec::seed("TEST", 256);
    assert_eq!(hamming_distance(&v, &v), 0);
}

#[test]
fn test_threshold_sum_majority() {
    let mut a = BitVec::new(64);
    a.set_bit(0);
    let mut b = BitVec::new(64);
    b.set_bit(1);
    let res = threshold_sum(&a, &b, 0.5);
    assert!(res.get_bit(0));
    assert!(res.get_bit(1));
    assert!(!res.get_bit(2));
}
