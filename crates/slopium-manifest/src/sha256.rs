//! SHA-256, written here rather than taken as a dependency (`D-037`).
//!
//! The build cache hashes with FNV-1a and says in its own doc comment that it
//! is a freshness check and not a security boundary. A lockfile checksum is a
//! different job: it has to survive someone who *wants* two different archives
//! to hash alike. This is the one place in the toolchain that needs a
//! cryptographic digest, and like the object writer it is checked against the
//! platform tool rather than trusted (`D-029`) — see `sha256sum_agrees` in the
//! tests and `scripts/package-check.sh`.

use std::fmt;

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// A 32-byte SHA-256 digest.
///
/// Rendered and parsed as lowercase hex, which is the form the lockfile and the
/// content-addressed store use.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() != 64 {
            return Err(format!(
                "invalid checksum `{text}`; expected 64 lowercase hex digits"
            ));
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let digits = &text[index * 2..index * 2 + 2];
            *byte = u8::from_str_radix(digits, 16)
                .map_err(|_| format!("invalid checksum `{text}`; `{digits}` is not hex"))?;
            if digits.chars().any(|digit| digit.is_ascii_uppercase()) {
                return Err(format!("invalid checksum `{text}`; use lowercase hex"));
            }
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Streaming SHA-256 state.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; 64],
            buffered: 0,
            length: 0,
        }
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        let mut rest = bytes;
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(rest.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&rest[..take]);
            self.buffered += take;
            rest = &rest[take..];
            // Still short of a block: the leftover is already where it belongs,
            // and falling through would overwrite `buffered` with zero.
            if self.buffered < 64 {
                return;
            }
            let block = self.buffer;
            self.compress(&block);
            self.buffered = 0;
        }
        while rest.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&rest[..64]);
            self.compress(&block);
            rest = &rest[64..];
        }
        self.buffer[..rest.len()].copy_from_slice(rest);
        self.buffered = rest.len();
    }

    pub fn finish(mut self) -> Digest {
        let bit_length = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        // `update` moved `length` on, but only the pre-padding length is hashed
        // into the trailing block, so it was captured above.
        while self.buffered != 56 {
            self.update(&[0x00]);
        }
        let block = {
            let mut block = self.buffer;
            block[56..].copy_from_slice(&bit_length.to_be_bytes());
            block
        };
        self.compress(&block);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Digest(digest)
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let previous = schedule[index - 15];
            let ahead = schedule[index - 2];
            let s0 = previous.rotate_right(7) ^ previous.rotate_right(18) ^ (previous >> 3);
            let s1 = ahead.rotate_right(17) ^ ahead.rotate_right(19) ^ (ahead >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

/// Digest of a single buffer.
pub fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three vectors every SHA-256 implementation is expected to reproduce.
    #[test]
    fn matches_published_vectors() {
        assert_eq!(
            sha256(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").to_string(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// A million `a`s crosses many blocks and the length-encoding boundary.
    #[test]
    fn matches_the_long_vector() {
        let mut hasher = Sha256::new();
        for _ in 0..1000 {
            hasher.update(&[b'a'; 1000]);
        }
        assert_eq!(
            hasher.finish().to_string(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// Feeding the same bytes in different chunk sizes must not change the
    /// answer — the buffering path is where a streaming hash usually breaks.
    #[test]
    fn chunking_does_not_change_the_digest() {
        let data = (0..1000u32)
            .flat_map(|value| value.to_be_bytes())
            .collect::<Vec<_>>();
        let whole = sha256(&data);
        for chunk in [1usize, 3, 55, 64, 65, 128, 1000] {
            let mut hasher = Sha256::new();
            for piece in data.chunks(chunk) {
                hasher.update(piece);
            }
            assert_eq!(hasher.finish(), whole, "chunk size {chunk}");
        }
    }

    #[test]
    fn digests_round_trip_through_hex() {
        let digest = sha256(b"slopium");
        assert_eq!(Digest::parse(&digest.to_string()).unwrap(), digest);
        assert!(Digest::parse("short").is_err());
        assert!(Digest::parse(&"A".repeat(64)).is_err());
    }

    /// `D-029`'s rule applied to the hash: check it against the platform tool
    /// rather than trusting it. Skipped where `sha256sum` is absent.
    #[test]
    fn sha256sum_agrees() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let payload = b"the object writer is checked against as, and this against sha256sum";
        let Ok(mut child) = Command::new("sha256sum")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        else {
            eprintln!("sha256sum not available; cross-check skipped");
            return;
        };
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(payload)
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let expected = String::from_utf8(output.stdout).unwrap();
        let expected = expected.split_whitespace().next().unwrap().to_owned();
        assert_eq!(sha256(payload).to_string(), expected);
    }
}
