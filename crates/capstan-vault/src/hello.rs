//! Optional, device-bound Windows Hello wrapping. The main envelope stays unchanged.

use crate::{
    envelope::{self, EnvelopeKind, Opened, HEADER_BYTES},
    VaultData,
};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use thiserror::Error;
use zeroize::Zeroizing;

pub const SIDECAR_FILE_NAME: &str = "vault-hello.cwh";
pub const MAX_SIDECAR_BYTES: usize = 1024;
const MAGIC: &[u8; 8] = b"CPHELLO1";
const VERSION: u16 = 1;
const KEY_PREFIX: &str = "Capstan.Hello.";
const KEY_SLOT_BYTES: usize = 104;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HelloError {
    #[error("Windows Hello is unavailable")]
    Unavailable,
    #[error("Windows Hello was canceled")]
    Canceled,
    #[error("Windows Hello key is missing")]
    KeyMissing,
    #[error("Windows Hello belongs to another vault")]
    Mismatch,
    #[error("Windows Hello data authentication failed")]
    Data,
}
impl HelloError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "hello_unavailable",
            Self::Canceled => "hello_canceled",
            Self::KeyMissing => "hello_key_missing",
            Self::Mismatch => "hello_mismatch",
            Self::Data => "hello_data",
        }
    }
    pub fn invalidates(self) -> bool {
        matches!(self, Self::KeyMissing | Self::Mismatch | Self::Data)
    }
}
pub type HelloResult<T> = std::result::Result<T, HelloError>;

/// Implementations must require a fresh Hello gesture for every `unprotect` call.
/// `protect` creates a new device key; it must never replace another enrollment.
pub trait HelloProvider: Send + Sync {
    fn available(&self) -> bool;
    fn protect(&self, name: &str, secret: &[u8; 32]) -> HelloResult<Vec<u8>>;
    fn unprotect(&self, name: &str, protected: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>>;
    fn exists(&self, name: &str) -> HelloResult<bool>;
    fn delete(&self, name: &str) -> HelloResult<()>;
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloStatus {
    pub available: bool,
    pub enabled: bool,
    pub invalidated: bool,
}

pub fn sidecar_path(vault: &Path) -> PathBuf {
    vault
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SIDECAR_FILE_NAME)
}
pub(crate) fn random<const N: usize>() -> HelloResult<[u8; N]> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| HelloError::Unavailable)?;
    Ok(bytes)
}
fn encrypt(key: &[u8; 32], value: &[u8; 32], aad: &[u8]) -> HelloResult<Vec<u8>> {
    let nonce = random::<24>()?;
    let mut result = nonce.to_vec();
    result.extend(
        XChaCha20Poly1305::new(key.into())
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: value, aad })
            .map_err(|_| HelloError::Data)?,
    );
    Ok(result)
}
fn decrypt(key: &[u8; 32], bytes: &[u8], aad: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
    if bytes.len() != 72 {
        return Err(HelloError::Data);
    }
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad,
                },
            )
            .map_err(|_| HelloError::Data)?,
    );
    Ok(Zeroizing::new(
        plain.as_slice().try_into().map_err(|_| HelloError::Data)?,
    ))
}
fn binding(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(&bytes[28..60]).into()
}
fn valid_name(name: &str) -> bool {
    name.strip_prefix(KEY_PREFIX).is_some_and(|id| {
        uuid::Uuid::parse_str(id)
            .is_ok_and(|id_value| id_value.to_string() == id && id_value.get_version_num() == 4)
    })
}

/// Public metadata and independently authenticated KEK slots. No plaintext key is retained here.
pub(crate) struct Sidecar {
    pub name: String,
    prefix: Vec<u8>,
    protected: Vec<u8>,
    wrapped_dek: Vec<u8>,
    slots: Vec<Vec<u8>>,
}
impl Sidecar {
    pub fn read(path: &Path) -> HelloResult<Self> {
        let file = File::open(path).map_err(|_| HelloError::Data)?;
        let mut bytes = Vec::new();
        file.take(MAX_SIDECAR_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| HelloError::Data)?;
        Self::decode(&bytes)
    }
    fn decode(bytes: &[u8]) -> HelloResult<Self> {
        if bytes.len() > MAX_SIDECAR_BYTES
            || bytes.len() < 28
            || &bytes[..8] != MAGIC
            || bytes[8..10] != VERSION.to_le_bytes()
        {
            return Err(HelloError::Data);
        }
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let name_end = 28 + name_len;
        let name = std::str::from_utf8(bytes.get(28..name_end).ok_or(HelloError::Data)?)
            .map_err(|_| HelloError::Data)?
            .to_owned();
        if !valid_name(&name) {
            return Err(HelloError::Data);
        }
        let length = bytes.get(name_end..name_end + 2).ok_or(HelloError::Data)?;
        let protected_len = u16::from_le_bytes([length[0], length[1]]) as usize;
        if protected_len != 256 {
            return Err(HelloError::Data);
        }
        let prefix_end = name_end + 2 + protected_len;
        let protected = bytes
            .get(name_end + 2..prefix_end)
            .ok_or(HelloError::Data)?
            .to_vec();
        let wrapped_dek = bytes
            .get(prefix_end..prefix_end + 72)
            .ok_or(HelloError::Data)?
            .to_vec();
        let count = usize::from(*bytes.get(prefix_end + 72).ok_or(HelloError::Data)?);
        if !(1..=2).contains(&count) || bytes.len() != prefix_end + 73 + count * KEY_SLOT_BYTES {
            return Err(HelloError::Data);
        }
        Ok(Self {
            name,
            prefix: bytes[..prefix_end].to_vec(),
            protected,
            wrapped_dek,
            slots: bytes[prefix_end + 73..]
                .chunks_exact(KEY_SLOT_BYTES)
                .map(<[u8]>::to_vec)
                .collect(),
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.prefix.clone();
        bytes.extend(&self.wrapped_dek);
        bytes.push(self.slots.len() as u8);
        for slot in &self.slots {
            bytes.extend(slot);
        }
        bytes
    }
    fn check_id(&self, bytes: &[u8]) -> HelloResult<()> {
        if bytes.get(12..28) != Some(&self.prefix[10..26]) {
            return Err(HelloError::Mismatch);
        }
        Ok(())
    }
    fn slot_aad(&self, binding: &[u8]) -> Vec<u8> {
        let mut aad = self.prefix.clone();
        aad.extend(&self.wrapped_dek);
        aad.extend(b"Capstan Hello KEK v1\0");
        aad.extend(binding);
        aad
    }
    fn slot(&self, opened: &Opened) -> HelloResult<Vec<u8>> {
        let binding = binding(&opened.bytes);
        let mut result = binding.to_vec();
        result.extend(encrypt(&opened.dek, &opened.kek, &self.slot_aad(&binding))?);
        Ok(result)
    }
    fn kek(&self, dek: &[u8; 32], bytes: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
        self.check_id(bytes)?;
        let wanted = binding(bytes);
        let mut result = None;
        for slot in &self.slots {
            let key = decrypt(dek, &slot[32..], &self.slot_aad(&slot[..32]))?;
            if slot[..32] == wanted {
                result = Some(key);
            }
        }
        result.ok_or(HelloError::Data)
    }
    /// True when every KEK slot authenticates under this Vault's DEK, even if none matches
    /// the current password file (for example after restoring an older backup). The slot
    /// AAD covers the key name, so only such an enrollment may select its native key.
    pub fn authenticated_by(&self, opened: &Opened) -> bool {
        self.check_id(&opened.bytes).is_ok()
            && self
                .slots
                .iter()
                .all(|slot| decrypt(&opened.dek, &slot[32..], &self.slot_aad(&slot[..32])).is_ok())
    }
    pub fn validate(&self, opened: &Opened) -> HelloResult<()> {
        let key = self.kek(&opened.dek, &opened.bytes)?;
        if *key != *opened.kek {
            return Err(HelloError::Data);
        }
        Ok(())
    }
    pub fn stage_password_change(&mut self, current: &Opened, next: &Opened) -> HelloResult<()> {
        self.validate(current)?;
        self.slots = vec![self.slot(current)?, self.slot(next)?];
        Ok(())
    }
    pub fn enroll(opened: &Opened, provider: &dyn HelloProvider) -> HelloResult<Self> {
        if !provider.available() {
            return Err(HelloError::Unavailable);
        }
        let name = format!("{KEY_PREFIX}{}", uuid::Uuid::new_v4());
        let wrapping_key = Zeroizing::new(random()?);
        let protected = provider.protect(&name, &wrapping_key)?;
        let result = (|| {
            if !(32..=512).contains(&protected.len()) {
                return Err(HelloError::Data);
            }
            // Verify actual Hello decryption before committing anything to disk.
            if *provider.unprotect(&name, &protected)? != *wrapping_key {
                return Err(HelloError::Data);
            }
            let mut prefix = MAGIC.to_vec();
            prefix.extend(VERSION.to_le_bytes());
            prefix.extend(&opened.bytes[12..28]);
            prefix.extend((name.len() as u16).to_le_bytes());
            prefix.extend(name.as_bytes());
            prefix.extend((protected.len() as u16).to_le_bytes());
            prefix.extend(&protected);
            let wrapped_dek = encrypt(&wrapping_key, &opened.dek, &prefix)?;
            let mut sidecar = Self {
                name: name.clone(),
                prefix,
                protected,
                wrapped_dek,
                slots: vec![],
            };
            sidecar.slots.push(sidecar.slot(opened)?);
            Ok(sidecar)
        })();
        if result.is_err() {
            let _ = provider.delete(&name);
        }
        result
    }
    pub fn open(&self, bytes: Vec<u8>, provider: &dyn HelloProvider) -> HelloResult<Opened> {
        envelope::inspect(&bytes, EnvelopeKind::Vault).map_err(|_| HelloError::Data)?;
        self.check_id(&bytes)?;
        let wrapping = provider.unprotect(&self.name, &self.protected)?;
        let dek = decrypt(&wrapping, &self.wrapped_dek, &self.prefix)?;
        let kek = self.kek(&dek, &bytes)?;
        // Authenticate the password slot as well as the payload before making a writable Vault.
        let mut aad = bytes[..112].to_vec();
        aad.extend(Sha256::digest(&bytes[HEADER_BYTES..]));
        let mut wrapped = bytes[60..84].to_vec();
        wrapped.extend(&bytes[112..HEADER_BYTES]);
        if *decrypt(&kek, &wrapped, &aad)? != *dek {
            return Err(HelloError::Data);
        }
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new((&*dek).into())
                .decrypt(
                    XNonce::from_slice(&bytes[84..108]),
                    Payload {
                        msg: &bytes[HEADER_BYTES..],
                        aad: &bytes[..28],
                    },
                )
                .map_err(|_| HelloError::Data)?,
        );
        // Enabling Hello follows password unlock, which already migrated legacy payloads.
        if bytes[10..12] != 1_u16.to_le_bytes() {
            return Err(HelloError::Data);
        }
        let data: VaultData = serde_json::from_slice(&plain).map_err(|_| HelloError::Data)?;
        data.validate().map_err(|_| HelloError::Data)?;
        Ok(Opened {
            data,
            dek,
            kek,
            bytes,
            migrated: false,
        })
    }
}

/// Stateless platform bridge. Windows UI handles and key material never enter the WebView.
#[derive(Default)]
pub struct PlatformHello {
    pub parent: isize,
}
#[cfg(windows)]
impl HelloProvider for PlatformHello {
    fn available(&self) -> bool {
        capstan_native_windows::hello::available()
    }
    fn protect(&self, name: &str, secret: &[u8; 32]) -> HelloResult<Vec<u8>> {
        capstan_native_windows::hello::protect(name, secret, self.parent).map_err(native_error)
    }
    fn unprotect(&self, name: &str, protected: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
        capstan_native_windows::hello::unprotect(name, protected, self.parent).map_err(native_error)
    }
    fn exists(&self, name: &str) -> HelloResult<bool> {
        capstan_native_windows::hello::exists(name).map_err(native_error)
    }
    fn delete(&self, name: &str) -> HelloResult<()> {
        capstan_native_windows::hello::delete(name).map_err(native_error)
    }
}
#[cfg(windows)]
fn native_error(error: capstan_native_windows::hello::HelloError) -> HelloError {
    use capstan_native_windows::hello::HelloError as Native;
    match error {
        Native::Unavailable => HelloError::Unavailable,
        Native::Canceled => HelloError::Canceled,
        Native::KeyMissing => HelloError::KeyMissing,
        Native::Data => HelloError::Data,
    }
}
#[cfg(not(windows))]
impl HelloProvider for PlatformHello {
    fn available(&self) -> bool {
        false
    }
    fn protect(&self, _: &str, _: &[u8; 32]) -> HelloResult<Vec<u8>> {
        Err(HelloError::Unavailable)
    }
    fn unprotect(&self, _: &str, _: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
        Err(HelloError::Unavailable)
    }
    fn exists(&self, _: &str) -> HelloResult<bool> {
        Err(HelloError::Unavailable)
    }
    fn delete(&self, _: &str) -> HelloResult<()> {
        Err(HelloError::Unavailable)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{KdfParameters, SecretText, Vault, VaultError};
    use std::{collections::HashMap, fs, sync::Mutex};

    #[derive(Default)]
    pub(crate) struct FakeProvider {
        keys: Mutex<HashMap<String, Zeroizing<[u8; 32]>>>,
        failure: Mutex<Option<HelloError>>,
        unprotect_failure: Mutex<Option<HelloError>>,
    }
    fn random_test_ciphertext() -> Vec<u8> {
        random::<256>().unwrap().to_vec()
    }
    impl HelloProvider for FakeProvider {
        fn available(&self) -> bool {
            true
        }
        fn protect(&self, name: &str, value: &[u8; 32]) -> HelloResult<Vec<u8>> {
            self.check()?;
            if self.keys.lock().unwrap().contains_key(name) {
                return Err(HelloError::Data);
            }
            let blob = random_test_ciphertext();
            self.keys
                .lock()
                .unwrap()
                .insert(name.into(), Zeroizing::new(*value));
            Ok(blob)
        }
        fn unprotect(&self, name: &str, blob: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
            self.check()?;
            if let Some(error) = *self.unprotect_failure.lock().unwrap() {
                return Err(error);
            }
            let keys = self.keys.lock().unwrap();
            let secret = keys.get(name).ok_or(HelloError::KeyMissing)?;
            if blob.len() != 256 {
                return Err(HelloError::Data);
            }
            Ok(secret.clone())
        }
        fn exists(&self, name: &str) -> HelloResult<bool> {
            self.check()?;
            Ok(self.keys.lock().unwrap().contains_key(name))
        }
        fn delete(&self, name: &str) -> HelloResult<()> {
            self.check()?;
            self.keys.lock().unwrap().remove(name);
            Ok(())
        }
    }
    impl FakeProvider {
        fn check(&self) -> HelloResult<()> {
            self.failure.lock().unwrap().map_or(Ok(()), Err)
        }
    }
    fn password() -> SecretText {
        SecretText::new("test Hello master password".into())
    }
    fn fast() -> KdfParameters {
        KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }
    #[test]
    fn hello_roundtrip_can_write_and_keeps_master_password_usable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        let original = fs::read(&path).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        vault.lock().unwrap();
        let unlocked = Vault::unlock_with_hello(&path, &provider).unwrap();
        unlocked
            .transaction(0, |data| {
                data.preferences.auto_lock_minutes = Some(12);
                Ok(())
            })
            .unwrap();
        let master = Vault::unlock(&path, &password()).unwrap();
        assert_eq!(
            master.snapshot().unwrap().preferences.auto_lock_minutes,
            Some(12)
        );
        unlocked.disable_hello(&provider).unwrap();
        assert!(!sidecar_path(&path).exists());
        assert!(provider.keys.lock().unwrap().is_empty());
        assert!(Vault::unlock_with_hello(&path, &provider).is_err());
        assert!(Vault::unlock(&path, &password()).is_ok());
    }
    #[test]
    fn hello_requires_the_current_master_password_and_never_changes_the_vault_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(matches!(
            vault.enable_hello(&SecretText::new("wrong".into()), &provider),
            Err(VaultError::Authentication)
        ));
        assert!(!sidecar_path(&path).exists());
        vault.enable_hello(&password(), &provider).unwrap();
        for error in [
            HelloError::Canceled,
            HelloError::Unavailable,
            HelloError::KeyMissing,
        ] {
            *provider.failure.lock().unwrap() = Some(error);
            assert_eq!(
                Vault::unlock_with_hello(&path, &provider)
                    .err()
                    .unwrap()
                    .code(),
                error.code()
            );
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(Vault::unlock(&path, &password()).is_ok());
        }
    }
    #[test]
    fn every_sidecar_byte_is_bounded_and_authenticated() {
        let provider = FakeProvider::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        let original = fs::read(&path).unwrap();
        let sidecar = sidecar_path(&path);
        let bytes = fs::read(&sidecar).unwrap();
        for offset in 0..bytes.len() {
            let mut tampered = bytes.clone();
            tampered[offset] ^= 1;
            fs::write(&sidecar, tampered).unwrap();
            assert!(
                Vault::unlock_with_hello(&path, &provider).is_err(),
                "offset {offset}"
            );
        }
        for size in 0..bytes.len() {
            fs::write(&sidecar, &bytes[..size]).unwrap();
            assert!(Vault::unlock_with_hello(&path, &provider).is_err());
        }
        fs::write(&sidecar, vec![0; MAX_SIDECAR_BYTES + 1]).unwrap();
        assert!(Vault::unlock_with_hello(&path, &provider).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(Vault::unlock(&path, &password()).is_ok());
    }
    #[test]
    fn password_changes_preserve_hello_and_the_encrypted_payload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        let original = fs::read(&path).unwrap();
        let next_password = SecretText::new("replacement master password".into());
        vault
            .change_master_password(0, &next_password, fast())
            .unwrap();
        let next = fs::read(&path).unwrap();
        assert_eq!(original[84..108], next[84..108]);
        assert_eq!(original[160..], next[160..]);
        assert!(Vault::unlock(&path, &password()).is_err());
        let unlocked = Vault::unlock_with_hello(&path, &provider).unwrap();
        unlocked.transaction(0, |_| Ok(())).unwrap();
        assert_eq!(
            Vault::unlock(&path, &next_password)
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            1
        );
        // Repeated changes keep only the current and the next KEK, never an unbounded history.
        unlocked
            .change_master_password(1, &password(), fast())
            .unwrap();
        assert_eq!(Sidecar::read(&sidecar_path(&path)).unwrap().slots.len(), 2);
        assert!(Vault::unlock_with_hello(&path, &provider).is_ok());
        assert!(Vault::unlock(&path, &password()).is_ok());
    }
    #[test]
    fn staged_password_change_survives_interruption_on_either_side_of_vault_replace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        let bytes = fs::read(&path).unwrap();
        let current = envelope::open(bytes.clone(), &password(), EnvelopeKind::Vault).unwrap();
        let mut next = current.fork();
        let new_password = SecretText::new("new password after interruption".into());
        envelope::rewrap(&mut next, &new_password, fast()).unwrap();
        let mut staged = Sidecar::read(&sidecar_path(&path)).unwrap();
        staged.stage_password_change(&current, &next).unwrap();
        fs::write(sidecar_path(&path), staged.encode()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(Vault::unlock_with_hello(&path, &provider).is_ok());
        assert!(Vault::unlock(&path, &password()).is_ok());
        fs::write(&path, &next.bytes).unwrap();
        let unlocked = Vault::unlock_with_hello(&path, &provider).unwrap();
        unlocked.transaction(0, |_| Ok(())).unwrap();
        assert!(Vault::unlock(&path, &new_password).is_ok());
    }
    #[test]
    fn invalid_enrollment_is_removed_only_after_password_unlock() {
        for mode in ["mismatch", "missing", "dek", "payload", "version"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("vault.capstan");
            let provider = FakeProvider::default();
            let vault = Vault::initialize(&path, &password(), fast()).unwrap();
            vault.enable_hello(&password(), &provider).unwrap();
            match mode {
                "mismatch" => {
                    let other = tempfile::tempdir().unwrap();
                    let other_path = other.path().join("vault.capstan");
                    Vault::initialize(&other_path, &password(), fast()).unwrap();
                    fs::copy(other_path, &path).unwrap();
                }
                "missing" => provider.keys.lock().unwrap().clear(),
                "dek" => {
                    let mut sidecar = Sidecar::read(&sidecar_path(&path)).unwrap();
                    let wrapping = provider
                        .unprotect(&sidecar.name, &sidecar.protected)
                        .unwrap();
                    sidecar.wrapped_dek =
                        encrypt(&wrapping, &random().unwrap(), &sidecar.prefix).unwrap();
                    fs::write(sidecar_path(&path), sidecar.encode()).unwrap();
                }
                "version" => {
                    let mut bytes = fs::read(sidecar_path(&path)).unwrap();
                    bytes[8] = 2;
                    fs::write(sidecar_path(&path), bytes).unwrap();
                }
                "payload" => {
                    let mut bytes = fs::read(&path).unwrap();
                    bytes[160] ^= 1;
                    fs::write(&path, bytes).unwrap();
                }
                _ => unreachable!(),
            }
            let original = fs::read(&path).unwrap();
            assert!(
                Vault::unlock_with_hello(&path, &provider).is_err(),
                "{mode}"
            );
            assert!(sidecar_path(&path).exists());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(Vault::unlock(&path, &SecretText::new("wrong".into())).is_err());
            assert!(sidecar_path(&path).exists());
            if mode != "payload" {
                let unlocked = Vault::unlock(&path, &password()).unwrap();
                assert!(unlocked.cleanup_invalid_hello(&provider).unwrap());
                assert!(!sidecar_path(&path).exists());
                assert_eq!(fs::read(&path).unwrap(), original);
            }
        }
    }
    #[test]
    fn temporary_provider_failure_keeps_enrollment_and_failed_enable_removes_new_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        for error in [HelloError::Unavailable, HelloError::Canceled] {
            *provider.failure.lock().unwrap() = Some(error);
            assert!(!vault.cleanup_invalid_hello(&provider).unwrap());
            assert!(sidecar_path(&path).exists());
        }
        *provider.failure.lock().unwrap() = None;
        vault.disable_hello(&provider).unwrap();
        fs::create_dir(sidecar_path(&path)).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(vault.enable_hello(&password(), &provider).is_err());
        assert!(provider.keys.lock().unwrap().is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(Vault::unlock(&path, &password()).is_ok());
    }
    #[test]
    fn disabling_removes_the_enrollment_even_when_the_device_provider_is_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        let original = fs::read(&path).unwrap();
        *provider.failure.lock().unwrap() = Some(HelloError::Unavailable);
        vault.disable_hello(&provider).unwrap();
        assert!(!sidecar_path(&path).exists());
        assert_eq!(fs::read(&path).unwrap(), original);
        *provider.failure.lock().unwrap() = None;
        assert!(Vault::unlock(&path, &password()).is_ok());
        assert!(Vault::unlock_with_hello(&path, &provider).is_err());
    }
    #[test]
    fn sidecar_is_excluded_from_recovery_and_independent_exports() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        assert!(Vault::recovery_candidates(&path, &password())
            .unwrap()
            .is_empty());
        vault.transaction(0, |_| Ok(())).unwrap();
        assert_eq!(
            Vault::recovery_candidates(&path, &password())
                .unwrap()
                .len(),
            1
        );
        let export = tempfile::tempdir().unwrap();
        let backup = export.path().join("export.cbak");
        vault.export_backup(&password(), fast(), &backup).unwrap();
        assert_eq!(fs::read_dir(export.path()).unwrap().count(), 1);
        assert_eq!(
            Vault::preview_backup(&backup, &password())
                .unwrap()
                .revision,
            1
        );
    }
    #[test]
    fn shared_contract_fixes_status_limits_and_error_codes() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/agent/fixtures/M4-contract.json"
        ))
        .unwrap();
        let status = HelloStatus {
            available: true,
            enabled: true,
            invalidated: false,
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            fixture["helloStatus"]
        );
        assert_eq!(fixture["helloSidecar"]["fileName"], SIDECAR_FILE_NAME);
        assert_eq!(fixture["helloSidecar"]["maxBytes"], MAX_SIDECAR_BYTES);
        assert_eq!(fixture["helloSidecar"]["version"], VERSION);
        assert_eq!(fixture["helloSidecar"]["maxKekSlots"], 2);
        assert_eq!(
            fixture["errorCodes"]["hello"],
            serde_json::json!([
                HelloError::Unavailable.code(),
                HelloError::Canceled.code(),
                HelloError::KeyMissing.code(),
                HelloError::Mismatch.code(),
                HelloError::Data.code()
            ])
        );
    }

    #[test]
    fn enrollment_cancellation_after_key_creation_keeps_the_previous_enrollment() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let provider = FakeProvider::default();
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault.enable_hello(&password(), &provider).unwrap();
        let original = fs::read(&path).unwrap();
        let sidecar = fs::read(sidecar_path(&path)).unwrap();
        *provider.unprotect_failure.lock().unwrap() = Some(HelloError::Canceled);
        assert_eq!(
            vault
                .enable_hello(&password(), &provider)
                .unwrap_err()
                .code(),
            "hello_canceled"
        );
        assert_eq!(provider.keys.lock().unwrap().len(), 1);
        assert_eq!(fs::read(sidecar_path(&path)).unwrap(), sidecar);
        assert_eq!(fs::read(&path).unwrap(), original);
        *provider.unprotect_failure.lock().unwrap() = None;
        assert!(Vault::unlock_with_hello(&path, &provider).is_ok());
    }

    #[test]
    fn independently_authenticated_but_wrong_dek_or_kek_cannot_unlock_the_main_envelope() {
        for wrong_dek in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("vault.capstan");
            let provider = FakeProvider::default();
            Vault::initialize(&path, &password(), fast()).unwrap();
            let original = fs::read(&path).unwrap();
            let mut opened =
                envelope::open(original.clone(), &password(), EnvelopeKind::Vault).unwrap();
            if wrong_dek {
                opened.dek = Zeroizing::new(random().unwrap());
            } else {
                opened.kek = Zeroizing::new(random().unwrap());
            }
            let sidecar = Sidecar::enroll(&opened, &provider).unwrap();
            fs::write(sidecar_path(&path), sidecar.encode()).unwrap();
            assert_eq!(
                Vault::unlock_with_hello(&path, &provider)
                    .err()
                    .unwrap()
                    .code(),
                "hello_data"
            );
            assert_eq!(fs::read(&path).unwrap(), original);
            let password_vault = Vault::unlock(&path, &password()).unwrap();
            assert!(password_vault.cleanup_invalid_hello(&provider).unwrap());
        }
    }
}
