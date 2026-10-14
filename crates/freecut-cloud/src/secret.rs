//! Credentials and where they are kept (CG-002, architecture §5.6).
//!
//! Tokens live only in [`Secret`]s, which never print their contents and
//! clear their memory when dropped (best effort, without `unsafe`). A
//! [`SecretStore`] keeps one [`Credential`] per provider; the app implements
//! it over the OS keychain through [`ChunkedSecretStore`], which splits long
//! tokens over several entries because Windows Credential Manager stores at
//! most 2,560 bytes per entry.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A token or other secret string.
///
/// `Debug` shows only its length, there is no `Display`, and the memory is
/// overwritten with zeros on drop. Copies made by callers through
/// [`Secret::expose`] are theirs to keep short-lived.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret value.
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The secret text, for building the one request that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True for an empty secret.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // `into_bytes` keeps the allocation, so this clears the bytes that
        // held the secret; `black_box` keeps the write from being elided.
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<redacted, {} bytes>)", self.0.len())
    }
}

/// Seconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UnixSeconds(pub i64);

/// The wall clock, injectable for tests.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current time.
    fn now(&self) -> UnixSeconds;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixSeconds {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        UnixSeconds(i64::try_from(secs).unwrap_or(i64::MAX))
    }
}

/// A generation service FreeCut can connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(&'static str);

impl ProviderId {
    /// Higgsfield's MCP server.
    pub const HIGGSFIELD: ProviderId = ProviderId("higgsfield");

    /// The id as stored in keychain entry names and the ledger.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The first 16 bytes of SHA-256 of the account's subject id: tags ledger
/// entries with the account (CG-002) without storing who it is.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubjectHash(pub [u8; 16]);

impl SubjectHash {
    /// Hashes a subject id (`sub` claim).
    pub fn of(subject: &str) -> SubjectHash {
        let digest = Sha256::digest(subject.as_bytes());
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest[..16]);
        SubjectHash(out)
    }

    /// Lower-case hex, 32 characters.
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Parses [`SubjectHash::to_hex`]'s output.
    pub fn from_hex(text: &str) -> Option<SubjectHash> {
        if text.len() != 32 || !text.is_ascii() {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        Some(SubjectHash(out))
    }
}

impl fmt::Debug for SubjectHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SubjectHash({})", self.to_hex())
    }
}

/// What FreeCut holds for one connected account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// Tokens from FreeCut's own sign-in (§5.3, §5.4).
    OAuth(OAuthCredential),
    /// A pasted access token: never renewed by FreeCut, expiry unknown (§5.5).
    Bearer {
        /// The token.
        token: Secret,
    },
}

impl Credential {
    /// The access token to send.
    pub fn access_token(&self) -> &Secret {
        match self {
            Credential::OAuth(c) => &c.access,
            Credential::Bearer { token } => token,
        }
    }
}

/// Tokens and metadata from an OAuth sign-in. `Debug` shows neither the
/// tokens, the client id nor the account e-mail (review SEC-6).
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthCredential {
    /// The access token.
    pub access: Secret,
    /// The refresh token, when the service issued one.
    pub refresh: Option<Secret>,
    /// When the access token expires, if the service said.
    pub expires_at: Option<UnixSeconds>,
    /// Granted scopes, space-separated.
    pub scope: String,
    /// FreeCut's registered client id (not secret; detects a mismatch with
    /// the preferences).
    pub client_id: String,
    /// The authorization server that issued the tokens; refreshes go there.
    pub issuer: String,
    /// The account tag, when the sign-in named a subject.
    pub subject_hash: Option<SubjectHash>,
    /// The account e-mail for the status line, when the sign-in returned one.
    pub account_label: Option<String>,
}

impl fmt::Debug for OAuthCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = |present: bool| if present { "<redacted>" } else { "<none>" };
        f.debug_struct("OAuthCredential")
            .field("access", &self.access)
            .field("refresh", &self.refresh)
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("client_id", &redacted(!self.client_id.is_empty()))
            .field("issuer", &self.issuer)
            .field("subject_hash", &self.subject_hash)
            .field("account_label", &redacted(self.account_label.is_some()))
            .finish()
    }
}

/// Why a credential store failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The store is missing, locked, or the user dismissed its prompt.
    #[error("the credential store is not available: {0}")]
    Unavailable(String),
    /// A value is larger than one entry may hold.
    #[error("the value is too large for one credential store entry")]
    TooLarge,
    /// The stored entries do not form a valid credential.
    #[error("the stored sign-in is damaged: {0}")]
    Corrupt(String),
}

/// Where credentials are kept, one per provider (architecture §5.6).
pub trait SecretStore: Send + Sync + fmt::Debug {
    /// The provider's credential, if one is stored.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be read or holds a damaged entry.
    fn get(&self, provider: ProviderId) -> Result<Option<Credential>, SecretError>;

    /// Stores (or replaces) the provider's credential.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be written; the previous
    /// credential then stays in place.
    fn set(&self, provider: ProviderId, credential: &Credential) -> Result<(), SecretError>;

    /// Removes the provider's credential. Removing nothing is not an error.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be written.
    fn delete(&self, provider: ProviderId) -> Result<(), SecretError>;
}

/// "Keep sign-in for this session only" (CG-002), and the store tests use.
#[derive(Debug, Default)]
pub struct SessionSecretStore {
    map: Mutex<BTreeMap<ProviderId, Credential>>,
}

impl SessionSecretStore {
    /// An empty store.
    pub fn new() -> SessionSecretStore {
        SessionSecretStore::default()
    }
}

impl SecretStore for SessionSecretStore {
    fn get(&self, provider: ProviderId) -> Result<Option<Credential>, SecretError> {
        let map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(map.get(&provider).cloned())
    }

    fn set(&self, provider: ProviderId, credential: &Credential) -> Result<(), SecretError> {
        let mut map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        map.insert(provider, credential.clone());
        Ok(())
    }

    fn delete(&self, provider: ProviderId) -> Result<(), SecretError> {
        let mut map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        map.remove(&provider);
        Ok(())
    }
}

/// The largest pasted token FreeCut accepts (§5.5).
pub const MAX_PASTED_TOKEN_BYTES: usize = 8 * 1024;

/// Why pasted text was refused before anything was stored or sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PastedTokenError {
    /// Nothing was pasted.
    #[error("the token is empty")]
    Empty,
    /// The text contains spaces, line breaks or other non-token characters.
    #[error("the token contains spaces, line breaks or other characters a token cannot have")]
    BadCharacters,
    /// Longer than [`MAX_PASTED_TOKEN_BYTES`].
    #[error("the token is longer than 8 KB")]
    TooLong,
}

/// Checks the shape of a pasted access token (CG-001): not empty, at most
/// 8 KB, printable ASCII without inner whitespace. Surrounding whitespace
/// from the clipboard is dropped.
///
/// # Errors
/// [`PastedTokenError`] for text that cannot be a token.
pub fn parse_pasted_token(text: &str) -> Result<Secret, PastedTokenError> {
    let token = text.trim();
    if token.is_empty() {
        return Err(PastedTokenError::Empty);
    }
    if token.len() > MAX_PASTED_TOKEN_BYTES {
        return Err(PastedTokenError::TooLong);
    }
    if !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(PastedTokenError::BadCharacters);
    }
    Ok(Secret::new(token))
}

/// One credential store entry per account name: what the app's keychain
/// type implements. Values are plain text.
pub trait EntryStore: Send + Sync + fmt::Debug {
    /// The entry's value, if it exists.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be read.
    fn read(&self, account: &str) -> Result<Option<String>, SecretError>;

    /// Creates or replaces an entry.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be written or the value is too
    /// large for one entry.
    fn write(&self, account: &str, value: &str) -> Result<(), SecretError>;

    /// Removes an entry; a missing entry is not an error.
    ///
    /// # Errors
    /// [`SecretError`] when the store cannot be written.
    fn remove(&self, account: &str) -> Result<(), SecretError>;
}

/// The longest value [`ChunkedSecretStore`] puts in one entry, in characters.
/// Windows stores entries as UTF-16 with a 2,560-byte limit (1,280
/// characters); the margin covers the store's own framing.
pub const MAX_ENTRY_CHARS: usize = 1_200;

/// Parts per token: 8 × 1,200 characters holds the largest pasted token.
const MAX_PARTS: usize = 8;

/// A [`SecretStore`] over plain entries: `<provider>/meta` plus numbered
/// parts `<provider>/access.<slot>.<n>` and `<provider>/refresh.<slot>.<n>`.
///
/// Every write uses the next generation number, whose parity is its slot,
/// so a new credential always goes to the slot the current one does not
/// use: all parts are written first, then `meta` (which names the
/// generation and part counts), then the other slot is cleared. A crash at
/// any point leaves either the old or the new credential readable, never a
/// mix; with only two slots, a delete can clear everything, including
/// parts a crash left before `meta` was written (review SEC-2).
pub struct ChunkedSecretStore<E> {
    entries: E,
    /// Readers share; `set` and `delete` are exclusive, so concurrent writes
    /// cannot mix parts and a read never sees parts being removed
    /// (review SEC-1). Another process is kept out by the ledger lock (H2).
    lock: RwLock<()>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    v: u32,
    generation: u64,
    kind: MetaKind,
    access_parts: usize,
    #[serde(default)]
    refresh_parts: usize,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    issuer: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MetaKind {
    Oauth,
    Bearer,
}

const META_VERSION: u32 = 1;
/// Longest account label kept in `meta`, in characters.
const MAX_LABEL_CHARS: usize = 256;

/// Shows nothing of the entries: they are token parts (review SEC-4).
impl<E> fmt::Debug for ChunkedSecretStore<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkedSecretStore").finish_non_exhaustive()
    }
}

impl<E: EntryStore> ChunkedSecretStore<E> {
    /// A store over `entries`.
    pub fn new(entries: E) -> ChunkedSecretStore<E> {
        ChunkedSecretStore {
            entries,
            lock: RwLock::new(()),
        }
    }

    /// The underlying entries (tests inspect them).
    pub fn entries(&self) -> &E {
        &self.entries
    }

    fn meta_account(provider: ProviderId) -> String {
        format!("{provider}/meta")
    }

    fn part_account(provider: ProviderId, which: &str, generation: u64, index: usize) -> String {
        format!("{provider}/{which}.{}.{index}", generation % 2)
    }

    fn read_meta(&self, provider: ProviderId) -> Result<Option<Meta>, SecretError> {
        let Some(text) = self.entries.read(&Self::meta_account(provider))? else {
            return Ok(None);
        };
        let meta: Meta = serde_json::from_str(&text)
            .map_err(|e| SecretError::Corrupt(format!("metadata: {e}")))?;
        if meta.v != META_VERSION {
            return Err(SecretError::Corrupt(format!(
                "unknown metadata version {}",
                meta.v
            )));
        }
        Ok(Some(meta))
    }

    fn read_parts(
        &self,
        provider: ProviderId,
        which: &str,
        generation: u64,
        count: usize,
    ) -> Result<Secret, SecretError> {
        if count == 0 || count > MAX_PARTS {
            return Err(SecretError::Corrupt(format!("{which}: {count} parts")));
        }
        let mut out = String::new();
        for index in 0..count {
            let part = self
                .entries
                .read(&Self::part_account(provider, which, generation, index))?
                .ok_or_else(|| SecretError::Corrupt(format!("{which}: part {index} missing")))?;
            out.push_str(&part);
            // Clears the intermediate copy on drop.
            drop(Secret::new(part));
        }
        Ok(Secret::new(out))
    }

    fn write_parts(
        &self,
        provider: ProviderId,
        which: &str,
        generation: u64,
        secret: &Secret,
    ) -> Result<usize, SecretError> {
        let parts = split_chars(secret.expose(), MAX_ENTRY_CHARS);
        if parts.is_empty() || parts.len() > MAX_PARTS {
            return Err(SecretError::TooLarge);
        }
        for (index, part) in parts.iter().enumerate() {
            self.entries.write(
                &Self::part_account(provider, which, generation, index),
                part,
            )?;
        }
        // Parts beyond the count may be left over from an interrupted write
        // of this generation; they are never read, but tidy them.
        for index in parts.len()..MAX_PARTS {
            let _ = self
                .entries
                .remove(&Self::part_account(provider, which, generation, index));
        }
        Ok(parts.len())
    }

    /// Removes every part in the slot of `generation`.
    fn clear_slot(&self, provider: ProviderId, generation: u64) {
        for index in 0..MAX_PARTS {
            for which in ["access", "refresh"] {
                if let Err(e) = self
                    .entries
                    .remove(&Self::part_account(provider, which, generation, index))
                {
                    tracing::debug!(error = %e, "could not remove an old credential part");
                }
            }
        }
    }

    /// The current metadata; a damaged one counts as none, an unreadable
    /// store is an error (it must not be mistaken for an empty one).
    fn meta_or_damaged(&self, provider: ProviderId) -> Result<Option<Meta>, SecretError> {
        match self.read_meta(provider) {
            Ok(meta) => Ok(meta),
            Err(SecretError::Corrupt(detail)) => {
                tracing::warn!(%detail, "the stored sign-in is damaged; it is replaced");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

/// Splits `text` into pieces of at most `max` characters, on char boundaries.
fn split_chars(text: &str, max: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() == max {
            parts.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

impl<E: EntryStore> SecretStore for ChunkedSecretStore<E> {
    fn get(&self, provider: ProviderId) -> Result<Option<Credential>, SecretError> {
        let _read = self.lock.read().unwrap_or_else(PoisonError::into_inner);
        let Some(meta) = self.read_meta(provider)? else {
            return Ok(None);
        };
        let access = self.read_parts(provider, "access", meta.generation, meta.access_parts)?;
        Ok(Some(match meta.kind {
            MetaKind::Bearer => Credential::Bearer { token: access },
            MetaKind::Oauth => {
                let refresh = if meta.refresh_parts == 0 {
                    None
                } else {
                    Some(self.read_parts(
                        provider,
                        "refresh",
                        meta.generation,
                        meta.refresh_parts,
                    )?)
                };
                Credential::OAuth(OAuthCredential {
                    access,
                    refresh,
                    expires_at: meta.expires_at.map(UnixSeconds),
                    scope: meta.scope,
                    client_id: meta.client_id,
                    issuer: meta.issuer,
                    subject_hash: meta.subject.as_deref().and_then(SubjectHash::from_hex),
                    account_label: meta.account,
                })
            }
        }))
    }

    fn set(&self, provider: ProviderId, credential: &Credential) -> Result<(), SecretError> {
        let _write = self.lock.write().unwrap_or_else(PoisonError::into_inner);
        // A damaged old entry must not block signing in again.
        let old = self.meta_or_damaged(provider)?;
        let generation = old.as_ref().map_or(1, |m| m.generation.wrapping_add(1));
        let access_parts =
            self.write_parts(provider, "access", generation, credential.access_token())?;
        let meta = match credential {
            Credential::Bearer { .. } => Meta {
                v: META_VERSION,
                generation,
                kind: MetaKind::Bearer,
                access_parts,
                refresh_parts: 0,
                expires_at: None,
                scope: String::new(),
                client_id: String::new(),
                issuer: String::new(),
                subject: None,
                account: None,
            },
            Credential::OAuth(c) => {
                let refresh_parts = match &c.refresh {
                    Some(refresh) => self.write_parts(provider, "refresh", generation, refresh)?,
                    None => 0,
                };
                Meta {
                    v: META_VERSION,
                    generation,
                    kind: MetaKind::Oauth,
                    access_parts,
                    refresh_parts,
                    expires_at: c.expires_at.map(|t| t.0),
                    scope: c.scope.clone(),
                    client_id: c.client_id.clone(),
                    issuer: c.issuer.clone(),
                    subject: c.subject_hash.map(SubjectHash::to_hex),
                    account: c
                        .account_label
                        .as_ref()
                        .map(|l| l.chars().take(MAX_LABEL_CHARS).collect()),
                }
            }
        };
        let text = serde_json::to_string(&meta)
            .map_err(|e| SecretError::Corrupt(format!("metadata: {e}")))?;
        self.entries.write(&Self::meta_account(provider), &text)?;
        // The other slot: the previous credential, or leftovers of a damaged
        // one or of an interrupted write.
        self.clear_slot(provider, generation.wrapping_add(1));
        Ok(())
    }

    fn delete(&self, provider: ProviderId) -> Result<(), SecretError> {
        let _write = self.lock.write().unwrap_or_else(PoisonError::into_inner);
        // An unreadable store is an error, not "nothing to delete".
        self.meta_or_damaged(provider)?;
        // Removing `meta` first makes the credential unreadable at once; then
        // both slots are cleared, so no token part survives.
        self.entries.remove(&Self::meta_account(provider))?;
        self.clear_slot(provider, 0);
        self.clear_slot(provider, 1);
        Ok(())
    }
}

/// An in-memory [`EntryStore`], optionally with Windows' entry size limit
/// and a write budget that simulates a crash.
#[derive(Default)]
pub struct MemoryEntryStore {
    inner: Mutex<MemoryEntries>,
}

#[derive(Default)]
struct MemoryEntries {
    map: BTreeMap<String, String>,
    max_utf16_bytes: Option<usize>,
    writes_left: Option<usize>,
    locked: bool,
}

/// Shows entry names only, never their values (review SEC-4).
impl fmt::Debug for MemoryEntryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryEntryStore")
            .field("accounts", &self.accounts())
            .finish_non_exhaustive()
    }
}

impl MemoryEntryStore {
    /// An unlimited store.
    pub fn new() -> MemoryEntryStore {
        MemoryEntryStore::default()
    }

    /// A store that refuses entries over Windows Credential Manager's
    /// 2,560 bytes of UTF-16.
    pub fn with_windows_limit() -> MemoryEntryStore {
        let store = MemoryEntryStore::default();
        store.lock().max_utf16_bytes = Some(2_560);
        store
    }

    /// After `writes` more successful writes, every write and remove fails,
    /// as if the process had died there.
    pub fn crash_after(&self, writes: usize) {
        self.lock().writes_left = Some(writes);
    }

    /// Writes and removes work again.
    pub fn recover(&self) {
        self.lock().writes_left = None;
    }

    /// While `locked`, every operation fails as a locked keychain does.
    pub fn set_locked(&self, locked: bool) {
        self.lock().locked = locked;
    }

    /// The names of all entries.
    pub fn accounts(&self) -> Vec<String> {
        self.lock().map.keys().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryEntries> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn spend_write(entries: &mut MemoryEntries) -> Result<(), SecretError> {
        match entries.writes_left.as_mut() {
            Some(0) => Err(SecretError::Unavailable("simulated crash".into())),
            Some(n) => {
                *n -= 1;
                Ok(())
            }
            None => Ok(()),
        }
    }
}

impl EntryStore for MemoryEntryStore {
    fn read(&self, account: &str) -> Result<Option<String>, SecretError> {
        let entries = self.lock();
        if entries.locked {
            return Err(SecretError::Unavailable("the keychain is locked".into()));
        }
        Ok(entries.map.get(account).cloned())
    }

    fn write(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let mut entries = self.lock();
        if entries.locked {
            return Err(SecretError::Unavailable("the keychain is locked".into()));
        }
        if let Some(limit) = entries.max_utf16_bytes
            && value.encode_utf16().count() * 2 > limit
        {
            return Err(SecretError::TooLarge);
        }
        Self::spend_write(&mut entries)?;
        entries.map.insert(account.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove(&self, account: &str) -> Result<(), SecretError> {
        let mut entries = self.lock();
        if entries.locked || entries.writes_left == Some(0) {
            return Err(SecretError::Unavailable("simulated crash".into()));
        }
        entries.map.remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn oauth(access_len: usize, refresh_len: usize, tag: char) -> Credential {
        Credential::OAuth(OAuthCredential {
            access: Secret::new(tag.to_string().repeat(access_len)),
            refresh: (refresh_len > 0).then(|| Secret::new("r".repeat(refresh_len))),
            expires_at: Some(UnixSeconds(1_800_000_000)),
            scope: "openid email offline_access".into(),
            client_id: "client-1".into(),
            issuer: "https://auth.example.test".into(),
            subject_hash: Some(SubjectHash::of("user_123")),
            account_label: Some("someone@example.test".into()),
        })
    }

    #[test]
    fn debug_never_shows_a_secret() {
        let s = Secret::new("very-secret-token");
        let shown = format!("{s:?} {:?}", Credential::Bearer { token: s.clone() });
        assert!(!shown.contains("very-secret"), "{shown}");
        assert!(shown.contains("17 bytes"));
    }

    /// SEC-4, SEC-6: debug output of stores and credentials shows no token
    /// part, client id or e-mail.
    #[test]
    fn debug_output_of_stores_and_credentials_is_redacted() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::new());
        let cred = oauth(1_500, 1_300, 'z');
        store.set(ProviderId::HIGGSFIELD, &cred).unwrap();
        let shown = format!("{store:?} {:?} {cred:?}", store.entries());
        for hidden in ["zzzz", "rrrr", "client-1", "someone@example.test"] {
            assert!(!shown.contains(hidden), "{hidden} in {shown}");
        }
        assert!(shown.contains("higgsfield/meta"), "{shown}");
    }

    #[test]
    fn subject_hash_round_trips_through_hex() {
        let h = SubjectHash::of("user_2abc");
        assert_eq!(h.to_hex().len(), 32);
        assert_eq!(SubjectHash::from_hex(&h.to_hex()), Some(h));
        assert_eq!(SubjectHash::from_hex("zz"), None);
    }

    #[test]
    fn pasted_tokens_are_checked_for_shape_only() {
        assert_eq!(
            parse_pasted_token("  abc.def-ghi_~+/=\n").unwrap().expose(),
            "abc.def-ghi_~+/="
        );
        assert_eq!(parse_pasted_token(" \n"), Err(PastedTokenError::Empty));
        assert_eq!(
            parse_pasted_token("Bearer abc"),
            Err(PastedTokenError::BadCharacters)
        );
        assert_eq!(
            parse_pasted_token("abc\ndef"),
            Err(PastedTokenError::BadCharacters)
        );
        assert_eq!(
            parse_pasted_token("tökén"),
            Err(PastedTokenError::BadCharacters)
        );
        assert_eq!(
            parse_pasted_token(&"a".repeat(MAX_PASTED_TOKEN_BYTES + 1)),
            Err(PastedTokenError::TooLong)
        );
        assert!(parse_pasted_token(&"a".repeat(MAX_PASTED_TOKEN_BYTES)).is_ok());
    }

    #[test]
    fn oversized_tokens_are_split_under_the_windows_limit_and_reassembled() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::with_windows_limit());
        // A JWT access token plus a refresh token well over 2,560 bytes.
        let cred = oauth(3_000, 1_500, 'a');
        store.set(ProviderId::HIGGSFIELD, &cred).unwrap();
        assert_eq!(store.get(ProviderId::HIGGSFIELD).unwrap(), Some(cred));
        let accounts = store.entries().accounts();
        assert!(
            accounts.contains(&"higgsfield/meta".to_string()),
            "{accounts:?}"
        );
        assert!(
            accounts.contains(&"higgsfield/access.1.2".to_string()),
            "{accounts:?}"
        );
        assert!(
            accounts.contains(&"higgsfield/refresh.1.1".to_string()),
            "{accounts:?}"
        );
    }

    #[test]
    fn the_largest_pasted_token_fits() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::with_windows_limit());
        let cred = Credential::Bearer {
            token: Secret::new("t".repeat(MAX_PASTED_TOKEN_BYTES)),
        };
        store.set(ProviderId::HIGGSFIELD, &cred).unwrap();
        assert_eq!(store.get(ProviderId::HIGGSFIELD).unwrap(), Some(cred));
    }

    #[test]
    fn replacing_a_credential_removes_the_old_parts() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::new());
        store
            .set(ProviderId::HIGGSFIELD, &oauth(2_500, 10, 'a'))
            .unwrap();
        let second = oauth(100, 10, 'b');
        store.set(ProviderId::HIGGSFIELD, &second).unwrap();
        assert_eq!(store.get(ProviderId::HIGGSFIELD).unwrap(), Some(second));
        assert_eq!(
            store.entries().accounts(),
            vec![
                "higgsfield/access.0.0",
                "higgsfield/meta",
                "higgsfield/refresh.0.0"
            ]
        );
    }

    #[test]
    fn a_crash_at_any_write_leaves_the_old_or_the_new_credential() {
        let old = oauth(2_600, 1_300, 'o');
        let new = oauth(2_700, 1_400, 'n');
        for crash_at in 0..12 {
            let store = ChunkedSecretStore::new(MemoryEntryStore::with_windows_limit());
            store.set(ProviderId::HIGGSFIELD, &old).unwrap();
            store.entries().crash_after(crash_at);
            let result = store.set(ProviderId::HIGGSFIELD, &new);
            store.entries().recover();
            let read = store.get(ProviderId::HIGGSFIELD).unwrap();
            if result.is_ok() {
                assert_eq!(read.as_ref(), Some(&new), "crash_at {crash_at}");
            } else {
                assert!(
                    read == Some(old.clone()) || read == Some(new.clone()),
                    "crash_at {crash_at}"
                );
            }
            // A delete right after the crash leaves no token part behind,
            // including parts written before the crash reached `meta`.
            store.delete(ProviderId::HIGGSFIELD).unwrap();
            assert!(store.entries().accounts().is_empty(), "crash_at {crash_at}");
            // The next write after a crash still succeeds.
            store.set(ProviderId::HIGGSFIELD, &new).unwrap();
            assert_eq!(
                store.get(ProviderId::HIGGSFIELD).unwrap(),
                Some(new.clone())
            );
        }
    }

    /// SEC-2: damaged metadata is cleared completely; a locked store is an
    /// error for both reads and deletes, never "nothing stored".
    #[test]
    fn damaged_metadata_is_swept_and_a_locked_store_is_an_error() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::new());
        store
            .set(ProviderId::HIGGSFIELD, &oauth(1_500, 1_300, 'a'))
            .unwrap();
        store
            .entries()
            .write("higgsfield/meta", "{not json")
            .unwrap();
        assert!(matches!(
            store.get(ProviderId::HIGGSFIELD),
            Err(SecretError::Corrupt(_))
        ));
        store.delete(ProviderId::HIGGSFIELD).unwrap();
        assert!(store.entries().accounts().is_empty());

        store
            .set(ProviderId::HIGGSFIELD, &oauth(100, 10, 'b'))
            .unwrap();
        store.entries().set_locked(true);
        assert!(matches!(
            store.get(ProviderId::HIGGSFIELD),
            Err(SecretError::Unavailable(_))
        ));
        assert!(matches!(
            store.delete(ProviderId::HIGGSFIELD),
            Err(SecretError::Unavailable(_))
        ));
        assert!(matches!(
            store.set(ProviderId::HIGGSFIELD, &oauth(100, 10, 'c')),
            Err(SecretError::Unavailable(_))
        ));
        store.entries().set_locked(false);
        assert_eq!(
            store.get(ProviderId::HIGGSFIELD).unwrap(),
            Some(oauth(100, 10, 'b'))
        );
    }

    /// SEC-1: writers are exclusive, so concurrent `set`s never mix parts and
    /// a reader never sees parts being removed.
    #[test]
    fn concurrent_writes_and_reads_never_mix_or_tear() {
        let store = std::sync::Arc::new(ChunkedSecretStore::new(
            MemoryEntryStore::with_windows_limit(),
        ));
        let a = oauth(3_000, 1_300, 'a');
        let b = oauth(2_500, 1_250, 'b');
        store.set(ProviderId::HIGGSFIELD, &a).unwrap();
        let writers: Vec<_> = [a.clone(), b.clone()]
            .into_iter()
            .map(|cred| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        store.set(ProviderId::HIGGSFIELD, &cred).unwrap();
                    }
                })
            })
            .collect();
        let reader = {
            let (store, a, b) = (store.clone(), a.clone(), b.clone());
            std::thread::spawn(move || {
                for _ in 0..400 {
                    let read = store.get(ProviderId::HIGGSFIELD).unwrap();
                    assert!(read == Some(a.clone()) || read == Some(b.clone()));
                }
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        reader.join().unwrap();
    }

    #[test]
    fn delete_makes_the_credential_unreadable_and_removes_its_parts() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::new());
        store
            .set(ProviderId::HIGGSFIELD, &oauth(1_300, 5, 'a'))
            .unwrap();
        store.delete(ProviderId::HIGGSFIELD).unwrap();
        assert_eq!(store.get(ProviderId::HIGGSFIELD).unwrap(), None);
        assert!(store.entries().accounts().is_empty());
        // Deleting nothing is fine.
        store.delete(ProviderId::HIGGSFIELD).unwrap();
    }

    #[test]
    fn a_missing_part_is_reported_as_damage() {
        let store = ChunkedSecretStore::new(MemoryEntryStore::new());
        store
            .set(ProviderId::HIGGSFIELD, &oauth(1_300, 0, 'a'))
            .unwrap();
        store.entries().remove("higgsfield/access.1.1").unwrap();
        assert!(matches!(
            store.get(ProviderId::HIGGSFIELD),
            Err(SecretError::Corrupt(_))
        ));
    }

    #[test]
    fn split_chars_respects_the_limit() {
        assert_eq!(split_chars("abcde", 2), vec!["ab", "cd", "e"]);
        assert_eq!(split_chars("ab", 2), vec!["ab"]);
        assert!(split_chars("", 2).is_empty());
    }
}
