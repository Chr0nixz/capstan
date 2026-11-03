use crate::{Result, SecretText, VaultData, VaultError};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const HEADER_BYTES: usize = 160;
const PREFIX_BYTES: usize = 112;
const VAULT_MAGIC: &[u8; 8] = b"CAPSTAN1";
const BACKUP_MAGIC: &[u8; 8] = b"CBSTBAK1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeKind {
    Vault,
    Backup,
}
impl EnvelopeKind {
    fn magic(self) -> &'static [u8; 8] {
        match self {
            Self::Vault => VAULT_MAGIC,
            Self::Backup => BACKUP_MAGIC,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KdfParameters {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}
impl Default for KdfParameters {
    fn default() -> Self {
        Self {
            memory_kib: 65_536,
            iterations: 3,
            parallelism: 1,
        }
    }
}
impl KdfParameters {
    pub fn validate(self) -> Result<()> {
        if !(19_456..=262_144).contains(&self.memory_kib)
            || !(2..=6).contains(&self.iterations)
            || !(1..=4).contains(&self.parallelism)
            || u64::from(self.memory_kib) * u64::from(self.iterations) > 1_048_576
        {
            return Err(VaultError::KdfPolicy);
        }
        Ok(())
    }
}

pub(crate) struct Opened {
    pub data: VaultData,
    pub dek: Zeroizing<[u8; 32]>,
    pub kek: Zeroizing<[u8; 32]>,
    pub bytes: Vec<u8>,
    pub migrated: bool,
}
impl Opened {
    pub fn fork(&self) -> Self {
        Self {
            data: self.data.clone(),
            dek: Zeroizing::new(*self.dek),
            kek: Zeroizing::new(*self.kek),
            bytes: self.bytes.clone(),
            migrated: self.migrated,
        }
    }
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut value = [0; N];
    OsRng
        .try_fill_bytes(&mut value)
        .map_err(|_| VaultError::Random)?;
    Ok(value)
}
fn number(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(VaultError::InvalidFormat)?
        .try_into()
        .map_err(|_| VaultError::InvalidFormat)?;
    Ok(u32::from_le_bytes(value))
}
pub(crate) fn inspect(bytes: &[u8], kind: EnvelopeKind) -> Result<KdfParameters> {
    if !(HEADER_BYTES + 16..=MAX_FILE_BYTES).contains(&bytes.len())
        || bytes.get(..8) != Some(kind.magic())
    {
        return Err(VaultError::InvalidFormat);
    }
    if bytes[8..10] != 1_u16.to_le_bytes() || u16::from_le_bytes([bytes[10], bytes[11]]) > 1 {
        return Err(VaultError::UnsupportedVersion);
    }
    if bytes[28..32] != [2, 19, 0, 0] || number(bytes, 108)? as usize != bytes.len() - HEADER_BYTES
    {
        return Err(VaultError::InvalidFormat);
    }
    let parameters = KdfParameters {
        memory_kib: number(bytes, 32)?,
        iterations: number(bytes, 36)?,
        parallelism: number(bytes, 40)?,
    };
    parameters.validate()?;
    Ok(parameters)
}
fn derive(
    password: &SecretText,
    salt: &[u8],
    parameters: KdfParameters,
) -> Result<Zeroizing<[u8; 32]>> {
    parameters.validate()?;
    if password.expose().len() > 1024 {
        return Err(VaultError::InvalidData);
    }
    let params = Params::new(
        parameters.memory_kib,
        parameters.iterations,
        parameters.parallelism,
        Some(32),
    )
    .map_err(|_| VaultError::KdfPolicy)?;
    let mut key = Zeroizing::new([0_u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.expose().as_bytes(), salt, key.as_mut())
        .map_err(|_| VaultError::Authentication)?;
    Ok(key)
}
fn configure(header: &mut [u8], parameters: KdfParameters) -> Result<()> {
    parameters.validate()?;
    header[28..32].copy_from_slice(&[2, 19, 0, 0]);
    header[32..36].copy_from_slice(&parameters.memory_kib.to_le_bytes());
    header[36..40].copy_from_slice(&parameters.iterations.to_le_bytes());
    header[40..44].copy_from_slice(&parameters.parallelism.to_le_bytes());
    header[44..60].copy_from_slice(&random::<16>()?);
    header[60..84].copy_from_slice(&random::<24>()?);
    Ok(())
}
fn wrapping_aad(bytes: &[u8]) -> Vec<u8> {
    let mut aad = bytes[..PREFIX_BYTES].to_vec();
    aad.extend_from_slice(&Sha256::digest(&bytes[HEADER_BYTES..]));
    aad
}
fn wrap(opened: &mut Opened) -> Result<()> {
    let cipher = XChaCha20Poly1305::new((&*opened.kek).into());
    let wrapped = cipher
        .encrypt(
            XNonce::from_slice(&opened.bytes[60..84]),
            Payload {
                msg: &*opened.dek,
                aad: &wrapping_aad(&opened.bytes),
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    opened.bytes[PREFIX_BYTES..HEADER_BYTES].copy_from_slice(&wrapped);
    Ok(())
}
pub(crate) fn reseal(opened: &mut Opened) -> Result<()> {
    opened.data.validate()?;
    let plain =
        Zeroizing::new(serde_json::to_vec(&opened.data).map_err(|_| VaultError::InvalidData)?);
    if plain.len() > MAX_FILE_BYTES - HEADER_BYTES - 16 {
        return Err(VaultError::InvalidData);
    }
    opened.bytes.truncate(HEADER_BYTES);
    opened.bytes[10..12].copy_from_slice(&1_u16.to_le_bytes());
    opened.bytes[60..84].copy_from_slice(&random::<24>()?);
    opened.bytes[84..108].copy_from_slice(&random::<24>()?);
    opened.bytes[108..112].copy_from_slice(&((plain.len() + 16) as u32).to_le_bytes());
    let cipher = XChaCha20Poly1305::new((&*opened.dek).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&opened.bytes[84..108]),
            Payload {
                msg: &plain,
                aad: &opened.bytes[..28],
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    opened.bytes.extend_from_slice(&ciphertext);
    wrap(opened)?;
    opened.migrated = false;
    Ok(())
}
pub(crate) fn seal(
    data: VaultData,
    password: &SecretText,
    parameters: KdfParameters,
    kind: EnvelopeKind,
) -> Result<Opened> {
    validate_new_password(password)?;
    let mut bytes = vec![0; HEADER_BYTES];
    bytes[..8].copy_from_slice(kind.magic());
    bytes[8..10].copy_from_slice(&1_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&1_u16.to_le_bytes());
    bytes[12..28].copy_from_slice(&random::<16>()?);
    configure(&mut bytes, parameters)?;
    let kek = derive(password, &bytes[44..60], parameters)?;
    let mut opened = Opened {
        data,
        dek: Zeroizing::new(random::<32>()?),
        kek,
        bytes,
        migrated: false,
    };
    reseal(&mut opened)?;
    Ok(opened)
}
pub(crate) fn rewrap(
    opened: &mut Opened,
    password: &SecretText,
    parameters: KdfParameters,
) -> Result<()> {
    validate_new_password(password)?;
    configure(&mut opened.bytes, parameters)?;
    opened.kek = derive(password, &opened.bytes[44..60], parameters)?;
    wrap(opened)
}
fn validate_new_password(password: &SecretText) -> Result<()> {
    if password.expose().chars().count() < 12 || password.expose().len() > 1024 {
        return Err(VaultError::InvalidData);
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyData {
    schema: u16,
    revision: u64,
    hosts: Vec<crate::Host>,
    groups: Vec<crate::Group>,
    credentials: Vec<crate::Credential>,
    trust: Vec<crate::TrustRecord>,
}
pub(crate) fn open(bytes: Vec<u8>, password: &SecretText, kind: EnvelopeKind) -> Result<Opened> {
    let parameters = inspect(&bytes, kind)?;
    let kek = derive(password, &bytes[44..60], parameters)?;
    let cipher = XChaCha20Poly1305::new((&*kek).into());
    let unwrapped = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[60..84]),
                Payload {
                    msg: &bytes[PREFIX_BYTES..HEADER_BYTES],
                    aad: &wrapping_aad(&bytes),
                },
            )
            .map_err(|_| VaultError::Authentication)?,
    );
    let dek: Zeroizing<[u8; 32]> = Zeroizing::new(
        unwrapped
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::InvalidFormat)?,
    );
    let cipher = XChaCha20Poly1305::new((&*dek).into());
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[84..108]),
                Payload {
                    msg: &bytes[HEADER_BYTES..],
                    aad: &bytes[..28],
                },
            )
            .map_err(|_| VaultError::Authentication)?,
    );
    let migrated = bytes[10..12] == [0, 0];
    let data = if migrated {
        let old: LegacyData =
            serde_json::from_slice(&plain).map_err(|_| VaultError::InvalidData)?;
        if old.schema != 0 {
            return Err(VaultError::InvalidData);
        }
        VaultData {
            schema: 1,
            revision: old.revision,
            hosts: old.hosts,
            groups: old.groups,
            credentials: old.credentials,
            trust: old.trust,
            preferences: crate::Preferences::default(),
            snippet_groups: Vec::new(),
            snippets: Vec::new(),
            tunnels: Vec::new(),
        }
    } else {
        serde_json::from_slice(&plain).map_err(|_| VaultError::InvalidData)?
    };
    data.validate()?;
    Ok(Opened {
        data,
        dek,
        kek,
        bytes,
        migrated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fast() -> KdfParameters {
        KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }
    fn password() -> SecretText {
        SecretText::new("testing master password".to_owned())
    }

    #[derive(Serialize)]
    struct LegacyFixture {
        schema: u16,
        revision: u64,
        hosts: Vec<crate::Host>,
        groups: Vec<crate::Group>,
        credentials: Vec<crate::Credential>,
        trust: Vec<crate::TrustRecord>,
    }

    fn legacy_schema_zero() -> Vec<u8> {
        let mut opened = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        let legacy = LegacyFixture {
            schema: 0,
            revision: 7,
            hosts: Vec::new(),
            groups: Vec::new(),
            credentials: Vec::new(),
            trust: Vec::new(),
        };
        let plaintext = Zeroizing::new(serde_json::to_vec(&legacy).unwrap());
        opened.bytes[10..12].copy_from_slice(&0_u16.to_le_bytes());
        opened.bytes[84..108].copy_from_slice(&random::<24>().unwrap());
        opened.bytes[108..112].copy_from_slice(&((plaintext.len() + 16) as u32).to_le_bytes());
        let cipher = XChaCha20Poly1305::new((&*opened.dek).into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&opened.bytes[84..108]),
                Payload {
                    msg: &plaintext,
                    aad: &opened.bytes[..28],
                },
            )
            .unwrap();
        opened.bytes.truncate(HEADER_BYTES);
        opened.bytes.extend_from_slice(&ciphertext);
        wrap(&mut opened).unwrap();
        opened.bytes
    }

    #[test]
    fn authenticated_schema_zero_fixture_migrates_in_memory() {
        let opened = open(legacy_schema_zero(), &password(), EnvelopeKind::Vault).unwrap();
        assert!(opened.migrated);
        assert_eq!(opened.data.schema, 1);
        assert_eq!(opened.data.revision, 7);
        assert_eq!(opened.data.preferences.auto_lock_minutes, Some(30));
        assert!(!opened.data.preferences.disconnect_on_lock);
        assert!(opened.data.snippet_groups.is_empty());
        assert!(opened.data.snippets.is_empty());
    }

    #[test]
    fn migrated_schema_zero_vault_reseals_with_empty_snippet_data() {
        let mut opened = open(legacy_schema_zero(), &password(), EnvelopeKind::Vault).unwrap();
        reseal(&mut opened).unwrap();

        let reopened = open(opened.bytes, &password(), EnvelopeKind::Vault).unwrap();
        assert!(!reopened.migrated);
        assert_eq!(reopened.data.revision, 7);
        assert!(reopened.data.snippet_groups.is_empty());
        assert!(reopened.data.snippets.is_empty());
    }

    #[test]
    fn roundtrip_authenticates_metadata_and_payload() {
        let source = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        assert_eq!(
            open(source.bytes.clone(), &password(), EnvelopeKind::Vault)
                .unwrap()
                .data
                .schema,
            1
        );
        assert!(matches!(
            open(
                source.bytes.clone(),
                &SecretText::new("incorrect password".into()),
                EnvelopeKind::Vault
            ),
            Err(VaultError::Authentication)
        ));
        for offset in [12, 44, 60, 84, 112, 160] {
            let mut changed = source.bytes.clone();
            changed[offset] ^= 1;
            assert!(
                matches!(
                    open(changed, &password(), EnvelopeKind::Vault),
                    Err(VaultError::Authentication)
                ),
                "offset {offset}"
            );
        }
    }
    #[test]
    fn untrusted_headers_are_bounded_before_derivation() {
        let source = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        let mut malicious = source.bytes.clone();
        malicious[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            open(malicious, &password(), EnvelopeKind::Vault),
            Err(VaultError::KdfPolicy)
        ));
        let mut unknown = source.bytes.clone();
        unknown[8] = 2;
        assert!(matches!(
            open(unknown, &password(), EnvelopeKind::Vault),
            Err(VaultError::UnsupportedVersion)
        ));
        for size in [0, 8, 159, source.bytes.len() - 1] {
            assert!(inspect(&source.bytes[..size], EnvelopeKind::Vault).is_err());
        }
        assert!(inspect(&source.bytes, EnvelopeKind::Backup).is_err());
    }
    #[test]
    fn master_change_only_rewraps_the_data_key() {
        let mut source = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        let immutable = source.bytes[..28].to_vec();
        let payload = source.bytes[84..112].to_vec();
        let ciphertext = source.bytes[160..].to_vec();
        let replacement = SecretText::new("replacement master password".into());
        rewrap(&mut source, &replacement, fast()).unwrap();
        assert_eq!(source.bytes[..28], immutable);
        assert_eq!(source.bytes[84..112], payload);
        assert_eq!(source.bytes[160..], ciphertext);
        assert!(open(source.bytes.clone(), &password(), EnvelopeKind::Vault).is_err());
        assert!(open(source.bytes, &replacement, EnvelopeKind::Vault).is_ok());
    }
    #[test]
    fn nonces_and_keys_are_fresh_for_every_write() {
        let mut first = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        let second = seal(
            VaultData::default(),
            &password(),
            fast(),
            EnvelopeKind::Vault,
        )
        .unwrap();
        assert_ne!(*first.dek, *second.dek);
        assert_ne!(first.bytes[44..108], second.bytes[44..108]);
        let before = first.bytes.clone();
        reseal(&mut first).unwrap();
        assert_ne!(first.bytes[60..108], before[60..108]);
    }
}
