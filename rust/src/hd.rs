use serde::{Serialize, Deserialize};

/// Simple 0/1 hyper dimensional vector stored as u64 lanes.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BitVec {
    pub dim: usize,
    pub lanes: Vec<u64>,
}

impl BitVec {
    /// Create an empty vector of given dimension
    pub fn new(dim: usize) -> Self {
        let lanes = vec![0u64; (dim + 63) / 64];
        BitVec { dim, lanes }
    }

    /// Generate a deterministic seed vector using FNV-1a hash and Xoshiro256+
    pub fn seed(label: &str, dim: usize) -> Self {
        let mut rng = Xoshiro256Plus::seed_from_u64(fnv1a_hash64(label));
        let mut lanes = vec![0u64; (dim + 63) / 64];
        for i in 0..dim {
            if rng.next_u64() & 1 == 1 {
                let lane = i / 64;
                let bit = i % 64;
                lanes[lane] |= 1u64 << bit;
            }
        }
        BitVec { dim, lanes }
    }

    /// Bitwise XOR producing a new vector
    pub fn xor(&self, other: &BitVec) -> BitVec {
        assert_eq!(self.dim, other.dim);
        let lanes = self
            .lanes
            .iter()
            .zip(other.lanes.iter())
            .map(|(a, b)| a ^ b)
            .collect();
        BitVec { dim: self.dim, lanes }
    }

    /// Rotate bits left by k positions
    pub fn rotate_left(&self, k: usize) -> BitVec {
        let shift = k % self.dim;
        if shift == 0 {
            return self.clone();
        }
        let mut res = Self::new(self.dim);
        for i in 0..self.dim {
            let src = (i + self.dim - shift) % self.dim;
            if self.get_bit(src) {
                res.set_bit(i);
            }
        }
        res
    }

    pub fn get_bit(&self, idx: usize) -> bool {
        let lane = idx / 64;
        let bit = idx % 64;
        (self.lanes[lane] >> bit) & 1 == 1
    }

    pub fn set_bit(&mut self, idx: usize) {
        let lane = idx / 64;
        let bit = idx % 64;
        self.lanes[lane] |= 1u64 << bit;
    }
}

/// Compute Hamming distance between two vectors
pub fn hamming_distance(a: &BitVec, b: &BitVec) -> usize {
    assert_eq!(a.dim, b.dim);
    a.lanes
        .iter()
        .zip(b.lanes.iter())
        .map(|(x, y)| (x ^ y).count_ones() as usize)
        .sum()
}

/// Simple majority/threshold bundling of two vectors
pub fn threshold_sum(a: &BitVec, b: &BitVec, theta: f32) -> BitVec {
    assert_eq!(a.dim, b.dim);
    let mut res = BitVec::new(a.dim);
    for i in 0..a.dim {
        let ones = a.get_bit(i) as u8 as f32 + b.get_bit(i) as u8 as f32;
        if ones / 2.0 >= theta {
            res.set_bit(i);
        }
    }
    res
}

/// FNV-1a 64-bit hash
fn fnv1a_hash64(text: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Xoshiro256+ pseudo random generator
#[derive(Clone)]
struct Xoshiro256Plus {
    s: [u64; 4],
}

impl Xoshiro256Plus {
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut x = seed;
        let mut s = [0u64; 4];
        for i in 0..4 {
            s[i] = splitmix64(&mut x);
        }
        Xoshiro256Plus { s }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[0].wrapping_add(self.s[3]);
        let t = self.s[1] << 17;

        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];

        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);

        result
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

