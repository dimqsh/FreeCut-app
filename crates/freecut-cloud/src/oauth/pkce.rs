//! PKCE (RFC 7636, S256) and the random `state` value (architecture §5.3).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::secret::Secret;

/// The OS random source failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no randomness available: {0}")]
pub struct RandomError(pub String);

/// A verifier that is not 43–128 unreserved characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a PKCE verifier must be 43 to 128 unreserved characters")]
pub struct BadVerifier;

/// A PKCE verifier and its S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    verifier: Secret,
    challenge: String,
}

impl Pkce {
    /// The `code_challenge_method` FreeCut uses.
    pub const METHOD: &'static str = "S256";

    /// A verifier from 64 random bytes (86 base64url characters).
    pub fn from_entropy(entropy: &[u8; 64]) -> Pkce {
        let verifier = URL_SAFE_NO_PAD.encode(entropy);
        let challenge = s256_challenge(&verifier);
        Pkce {
            verifier: Secret::new(verifier),
            challenge,
        }
    }

    /// Wraps a given verifier (tests and RFC vectors).
    ///
    /// # Errors
    /// [`BadVerifier`] when it is not 43–128 unreserved characters.
    pub fn from_verifier(verifier: Secret) -> Result<Pkce, BadVerifier> {
        let v = verifier.expose();
        let unreserved =
            |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if !(43..=128).contains(&v.len()) || !v.bytes().all(unreserved) {
            return Err(BadVerifier);
        }
        let challenge = s256_challenge(v);
        Ok(Pkce {
            verifier,
            challenge,
        })
    }

    /// A fresh verifier from the OS random source.
    ///
    /// # Errors
    /// [`RandomError`] when no randomness is available.
    pub fn generate() -> Result<Pkce, RandomError> {
        let mut entropy = [0u8; 64];
        getrandom::fill(&mut entropy).map_err(|e| RandomError(e.to_string()))?;
        let pkce = Pkce::from_entropy(&entropy);
        entropy.fill(0);
        Ok(pkce)
    }

    /// The verifier, sent with the code exchange.
    pub fn verifier(&self) -> &Secret {
        &self.verifier
    }

    /// The challenge, sent in the authorize URL.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

/// `BASE64URL(SHA256(ASCII(verifier)))` without padding.
pub fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A random `state` value: 32 bytes, base64url.
///
/// # Errors
/// [`RandomError`] when no randomness is available.
pub fn random_state() -> Result<Secret, RandomError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| RandomError(e.to_string()))?;
    let state = URL_SAFE_NO_PAD.encode(bytes);
    bytes.fill(0);
    Ok(Secret::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_7636_appendix_b_vector() {
        let pkce = Pkce::from_verifier(Secret::new("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
            .unwrap();
        assert_eq!(
            pkce.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifiers_from_entropy_are_valid_and_86_characters() {
        let pkce = Pkce::from_entropy(&[7u8; 64]);
        assert_eq!(pkce.verifier().len(), 86);
        assert!(Pkce::from_verifier(pkce.verifier().clone()).is_ok());
        assert_eq!(pkce.challenge(), s256_challenge(pkce.verifier().expose()));
    }

    #[test]
    fn generated_values_differ() {
        let a = Pkce::generate().unwrap();
        let b = Pkce::generate().unwrap();
        assert_ne!(a.verifier(), b.verifier());
        assert_ne!(random_state().unwrap(), random_state().unwrap());
        assert_eq!(random_state().unwrap().len(), 43);
    }

    #[test]
    fn bad_verifiers_are_refused() {
        assert_eq!(Pkce::from_verifier(Secret::new("short")), Err(BadVerifier));
        assert_eq!(
            Pkce::from_verifier(Secret::new("a".repeat(129))),
            Err(BadVerifier)
        );
        assert_eq!(
            Pkce::from_verifier(Secret::new(format!("{}+", "a".repeat(43)))),
            Err(BadVerifier)
        );
    }
}
