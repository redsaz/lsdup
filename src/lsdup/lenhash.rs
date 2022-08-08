use arrayvec::ArrayString;

#[derive(std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq, std::fmt::Debug)]
pub struct LenHash {
    len: u64,
    hash: [u8; 32],
}

impl LenHash {
    pub fn from(len: u64, hash: [u8; 32]) -> LenHash {
        LenHash { len, hash }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn to_hex(&self) -> ArrayString<64> {
        // As done in Blake3 to_hex function.
        let mut s = ArrayString::new();
        let table = b"0123456789abcdef";
        for &b in self.hash.iter() {
            s.push(table[(b >> 4) as usize] as char);
            s.push(table[(b & 0xf) as usize] as char);
        }
        s
    }
}

impl Ord for LenHash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare other with self, instead of self with other,
        // so the ordering becomes largest-to-smallest
        other
            .len
            .cmp(&self.len)
            .then_with(|| other.hash.cmp(&self.hash))
    }
}

impl PartialOrd for LenHash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
