//! Ed25519 over what a package claims to be (`D-056`).
//!
//! Everything here is text of the shape `<what>:<hex>`, because a key or a
//! signature is something a person copies between a terminal and a
//! configuration file and back:
//!
//! - `ed25519:<64 hex>` — a public key, which is what `trusted-keys` lists;
//! - `ed25519-private:<64 hex>` — a key file, and nothing else;
//! - `ed25519:<64 hex>:<128 hex>` — a signature and the key that claims to have
//!   made it, which is the index entry's `signature` field and the whole
//!   content of the `.sig` beside an archive.
//!
//! What gets signed is a statement rather than the digest alone. Two archives
//! with the same bytes are the same bytes, and an attacker picks the contents
//! of the package they publish — so a signature over a bare hash is one that
//! can be lifted onto another name or another version and still verify. Naming
//! the package inside the signed message is what stops that, and it costs a
//! newline.

use crate::sha256::Digest;
use crate::version::Version;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use std::fmt;
use std::io::Read;
use std::path::Path;

/// The prefix of a public key and of a signature.
pub const ALGORITHM: &str = "ed25519";
/// The prefix of a key file's single line.
pub const PRIVATE_ALGORITHM: &str = "ed25519-private";
/// What is appended to an archive's name to get its detached signature.
pub const SIGNATURE_EXTENSION: &str = "sig";
/// The first line of what is signed, so a second statement format is possible.
const STATEMENT: &str = "slopium-package-v1";

/// The exact bytes a signature is over.
///
/// A signature is an assertion that a named package at a named version archives
/// to a particular digest — not that some bytes exist.
pub fn statement(name: &str, version: &Version, digest: &Digest) -> Vec<u8> {
    format!("{STATEMENT}\n{name}\n{version}\n{digest}\n").into_bytes()
}

/// A public key, as `trusted-keys` writes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn parse(text: &str) -> Result<Self, String> {
        let hex = text.strip_prefix(&format!("{ALGORITHM}:")).ok_or_else(|| {
            format!("`{text}` is not a public key; one is written `{ALGORITHM}:<64 hex digits>`")
        })?;
        let bytes = decode::<32>(hex, "a public key")?;
        VerifyingKey::from_bytes(&bytes)
            .map(Self)
            .map_err(|error| format!("`{text}` is not a usable {ALGORITHM} key: {error}"))
    }

    fn verifies(&self, message: &[u8], signature: &ed25519_dalek::Signature) -> bool {
        // Strict verification: the permissive one accepts keys and signatures
        // in a small order subgroup, under which one signature can verify for
        // more than one key.
        self.0.verify_strict(message, signature).is_ok()
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{ALGORITHM}:{}", hex(self.0.as_bytes()))
    }
}

/// A signature and the key that claims to have made it.
///
/// The key travels with the signature because 64 opaque bytes cannot say who
/// produced them, and "a publisher you have not listed signed this" is a
/// different thing to be told than "this does not verify" (`D-056`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Signature {
    key: PublicKey,
    signature: ed25519_dalek::Signature,
}

impl Signature {
    pub fn parse(text: &str) -> Result<Self, String> {
        let malformed = || {
            format!("`{text}` is not a signature; one is written `{ALGORITHM}:<key>:<signature>`")
        };
        let rest = text
            .strip_prefix(&format!("{ALGORITHM}:"))
            .ok_or_else(malformed)?;
        let (key, signature) = rest.split_once(':').ok_or_else(malformed)?;
        let bytes = decode::<64>(signature, "a signature")?;
        Ok(Self {
            key: PublicKey::parse(&format!("{ALGORITHM}:{key}"))?,
            signature: ed25519_dalek::Signature::from_bytes(&bytes),
        })
    }

    /// The key this signature says made it — a claim, not a grant. It is worth
    /// nothing until it is found in `trusted-keys`.
    pub fn claimed_key(&self) -> &PublicKey {
        &self.key
    }

    /// Whether the claimed key really did sign this package at this version.
    pub fn verifies(&self, name: &str, version: &Version, digest: &Digest) -> bool {
        self.key
            .verifies(&statement(name, version, digest), &self.signature)
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}",
            self.key,
            hex(&self.signature.to_bytes())
        )
    }
}

/// A signing key, which lives in a file and never anywhere else (`D-060`).
///
/// Deliberately not `Debug`: the one thing a derived `Debug` would print is the
/// thing that must not be printed, and a panic message or a log line is not a
/// place to discover that.
pub struct PrivateKey(SigningKey);

impl PrivateKey {
    /// A new key from 32 bytes of `/dev/urandom`.
    ///
    /// The platform that has one is the only platform this toolchain targets,
    /// and reading it directly is what keeps `D-037`'s single new dependency to
    /// the signature scheme rather than to a random-number stack under it.
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut seed))
            .map_err(|error| format!("cannot read `/dev/urandom` for a new key: {error}"))?;
        Ok(Self(SigningKey::from_bytes(&seed)))
    }

    /// Read a key file, refusing one anybody else on the machine can read.
    pub fn read(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::PermissionsExt;
        let described = path.display();
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("cannot read the signing key `{described}`: {error}"))?;
        let mode = metadata.permissions().mode() & 0o077;
        if mode != 0 {
            return Err(format!(
                "the signing key `{described}` is mode {:04o}; it is readable by somebody other than you, so it is not a secret any more. Run `chmod 600 {described}` and, if anyone else has an account on this machine, publish under a new key",
                metadata.permissions().mode() & 0o7777
            ));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read the signing key `{described}`: {error}"))?;
        let hex = text
            .trim()
            .strip_prefix(&format!("{PRIVATE_ALGORITHM}:"))
            .ok_or_else(|| {
                format!(
                    "`{described}` does not hold a signing key; one line, `{PRIVATE_ALGORITHM}:<64 hex digits>`, is the whole format"
                )
            })?;
        let seed = decode::<32>(hex, "a signing key")?;
        Ok(Self(SigningKey::from_bytes(&seed)))
    }

    /// Write a key file at mode 0600, refusing to overwrite an existing one.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let described = path.display();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
        }
        // Exclusive, because the failure mode of overwriting is losing the key
        // every already-published signature was made with.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| format!("cannot create the signing key `{described}`: {error}"))?;
        writeln!(file, "{PRIVATE_ALGORITHM}:{}", hex(&self.0.to_bytes()))
            .map_err(|error| format!("cannot write `{described}`: {error}"))
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    pub fn sign(&self, name: &str, version: &Version, digest: &Digest) -> Signature {
        Signature {
            key: self.public(),
            signature: self.0.sign(&statement(name, version, digest)),
        }
    }
}

/// What the trusted keys of a registry say about one signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trust {
    /// A key on the list signed this package at this version.
    Signed,
    /// The signature names a key nobody listed. Its own message names the key,
    /// because adding it is what a rotation looks like from here.
    UnknownKey,
    /// The claimed key is trusted and did not sign this.
    Forged,
}

/// Whether a signature is one of these keys asserting this package.
///
/// The key is checked against the list *before* it verifies anything, so a
/// signature can never introduce the key that makes it acceptable.
pub fn trust(
    trusted: &[PublicKey],
    signature: &Signature,
    name: &str,
    version: &Version,
    digest: &Digest,
) -> Trust {
    if !trusted.contains(signature.claimed_key()) {
        return Trust::UnknownKey;
    }
    match signature.verifies(name, version, digest) {
        true => Trust::Signed,
        false => Trust::Forged,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

/// Exactly `N` bytes of lowercase hex, on the same terms as a digest: one hex
/// convention across the whole toolchain, so nothing here reads as valid in one
/// place and invalid in another.
fn decode<const N: usize>(text: &str, what: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2 {
        return Err(format!(
            "{what} is `{text}`; expected {} lowercase hex digits",
            N * 2
        ));
    }
    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let digits = &text[index * 2..index * 2 + 2];
        if digits.chars().any(|digit| digit.is_ascii_uppercase()) {
            return Err(format!("{what} is `{text}`; use lowercase hex"));
        }
        *byte = u8::from_str_radix(digits, 16)
            .map_err(|_| format!("{what} holds `{digits}`, which is not hex"))?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::sha256;

    fn key() -> PrivateKey {
        PrivateKey(SigningKey::from_bytes(&[7u8; 32]))
    }

    #[test]
    fn a_signature_verifies_what_it_signed() {
        let digest = sha256(b"geometry");
        let signature = key().sign("geometry", &Version::new(1, 4, 0), &digest);
        assert!(signature.verifies("geometry", &Version::new(1, 4, 0), &digest));
        assert_eq!(signature.claimed_key(), &key().public());
    }

    /// `D-056`: the whole reason the statement names the package. Identical
    /// bytes under another name must not carry the signature over.
    #[test]
    fn a_signature_does_not_travel_to_another_package() {
        let digest = sha256(b"geometry");
        let signature = key().sign("geometry", &Version::new(1, 4, 0), &digest);
        assert!(!signature.verifies("units", &Version::new(1, 4, 0), &digest));
        assert!(!signature.verifies("geometry", &Version::new(1, 5, 0), &digest));
        assert!(!signature.verifies("geometry", &Version::new(1, 4, 0), &sha256(b"other")));
    }

    #[test]
    fn keys_and_signatures_round_trip_through_text() {
        let signature = key().sign("geometry", &Version::new(1, 4, 0), &sha256(b"geometry"));
        assert_eq!(Signature::parse(&signature.to_string()).unwrap(), signature);
        let public = key().public();
        assert_eq!(PublicKey::parse(&public.to_string()).unwrap(), public);
        assert!(public.to_string().starts_with("ed25519:"));
    }

    /// A private key pasted where a public one goes is a mistake worth making
    /// loud, which is what the differing prefix is for.
    #[test]
    fn a_private_key_is_not_a_public_one() {
        let error = PublicKey::parse("ed25519-private:00").unwrap_err();
        assert!(error.contains("is not a public key"), "{error}");
    }

    /// The key in a signature is a claim. It must not be able to make itself
    /// acceptable by being present.
    #[test]
    fn a_signature_cannot_introduce_its_own_key() {
        let digest = sha256(b"geometry");
        let version = Version::new(1, 4, 0);
        let signature = key().sign("geometry", &version, &digest);
        assert_eq!(
            trust(&[], &signature, "geometry", &version, &digest),
            Trust::UnknownKey
        );
        assert_eq!(
            trust(&[key().public()], &signature, "geometry", &version, &digest),
            Trust::Signed
        );
    }

    /// A trusted key whose signature is for something else is a forgery, and
    /// is a different report from an unlisted key.
    #[test]
    fn a_trusted_key_that_did_not_sign_this_is_a_forgery() {
        let version = Version::new(1, 4, 0);
        let signature = key().sign("geometry", &version, &sha256(b"geometry"));
        assert_eq!(
            trust(
                &[key().public()],
                &signature,
                "geometry",
                &version,
                &sha256(b"tampered")
            ),
            Trust::Forged
        );
    }

    #[test]
    fn a_key_file_round_trips_and_refuses_a_readable_one() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("slopium-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("signing-key");
        key().write(&path).unwrap();
        assert_eq!(PrivateKey::read(&path).unwrap().public(), key().public());
        assert!(
            key().write(&path).is_err(),
            "an existing key is not overwritten"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        // Matched rather than unwrapped, because a key that could be unwrapped
        // out of a `Result` would be a key with a `Debug` that prints it.
        let Err(error) = PrivateKey::read(&path) else {
            panic!("a world-readable key file is refused");
        };
        assert!(
            error.contains("readable by somebody other than you"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
