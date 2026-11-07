//! Windows Hello RSA decryption through Microsoft Passport KSP only.
//! Unsupported enrollment/gesture policies fail closed; no software/DPAPI fallback.

use std::ptr::{null, null_mut};
use windows_sys::{core::w, Win32::Security::Cryptography::*};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloError {
    Unavailable,
    Canceled,
    KeyMissing,
    Data,
}
type Result<T> = std::result::Result<T, HelloError>;

const AUTH_MANDATORY: u32 = 1;
const CACHE_TYPE: windows_sys::core::PCWSTR = w!("NgcCacheType");
const SILENT_CONTEXT: i32 = 0x80090022_u32 as i32;

struct Handle(usize);
impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: this wrapper owns exactly one successful NCrypt handle.
            unsafe {
                NCryptFreeObject(self.0);
            }
        }
    }
}
fn error(status: i32) -> HelloError {
    match status as u32 {
        0x80090036 | 0x800704c7 | 0x80004004 => HelloError::Canceled,
        0x8009000d | 0x80090011 | 0x80090016 => HelloError::KeyMissing,
        0x80090005 => HelloError::Data,
        _ => HelloError::Unavailable,
    }
}
fn check(status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(error(status))
    }
}
fn provider() -> Result<Handle> {
    let mut handle = 0;
    // SAFETY: the constant is NUL-terminated; the output is initialized and owned on success.
    check(unsafe { NCryptOpenStorageProvider(&mut handle, MS_NGC_KEY_STORAGE_PROVIDER, 0) })?;
    Ok(Handle(handle))
}
pub fn available() -> bool {
    provider().is_ok()
}

fn valid_name(name: &str) -> bool {
    let Some(id) = name.strip_prefix("Capstan.Hello.") else {
        return false;
    };
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
}
fn key_name(name: &str) -> Result<Vec<u16>> {
    if !valid_name(name) {
        return Err(HelloError::Data);
    }
    // Passport KSP scopes persisted key names to the current user's SID.
    let sid = current_sid()?;
    Ok(format!("{sid}//Capstan/Vault/{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}
fn open(provider: &Handle, name: &str) -> Result<Handle> {
    let name = key_name(name)?;
    let mut key = 0;
    // SAFETY: provider is live and name/output stay alive for this synchronous call.
    check(unsafe { NCryptOpenKey(provider.0, &mut key, name.as_ptr(), 0, NCRYPT_SILENT_FLAG) })?;
    Ok(Handle(key))
}
fn set(key: &Handle, property: windows_sys::core::PCWSTR, bytes: &[u8]) -> Result<()> {
    // SAFETY: property is a static NUL-terminated name; bytes is valid for its given length.
    check(unsafe { NCryptSetProperty(key.0, property, bytes.as_ptr(), bytes.len() as u32, 0) })
}
fn context(key: &Handle, parent: isize) -> Result<()> {
    if parent != 0 {
        set(key, NCRYPT_WINDOW_HANDLE_PROPERTY, &parent.to_ne_bytes())?;
    }
    let context: Vec<u8> = "Capstan\0"
        .encode_utf16()
        .flat_map(u16::to_ne_bytes)
        .collect();
    set(key, NCRYPT_USE_CONTEXT_PROPERTY, &context)
}
fn get_dword(key: &Handle, property: windows_sys::core::PCWSTR) -> Result<u32> {
    let mut value = 0_u32;
    let mut size = 0;
    // SAFETY: the output is an aligned, initialized DWORD with an exact byte count.
    check(unsafe {
        NCryptGetProperty(
            key.0,
            property,
            (&mut value as *mut u32).cast(),
            4,
            &mut size,
            NCRYPT_SILENT_FLAG,
        )
    })?;
    if size != 4 {
        return Err(HelloError::Data);
    }
    Ok(value)
}
fn check_policy(key: &Handle) -> Result<()> {
    if get_dword(key, CACHE_TYPE)? != AUTH_MANDATORY
        || get_dword(key, NCRYPT_KEY_USAGE_PROPERTY)? != NCRYPT_ALLOW_DECRYPT_FLAG
        || get_dword(key, NCRYPT_EXPORT_POLICY_PROPERTY)? != 0
        || get_dword(key, NCRYPT_LENGTH_PROPERTY)? != 2048
    {
        return Err(HelloError::Data);
    }
    Ok(())
}
fn mandatory_gesture(key: &Handle) -> Result<()> {
    check_policy(key)?;
    set(
        key,
        NCRYPT_PIN_CACHE_IS_GESTURE_REQUIRED_PROPERTY,
        &1_u32.to_ne_bytes(),
    )
}
fn require_interaction(silent_status: i32) -> Result<()> {
    if silent_status == SILENT_CONTEXT {
        Ok(())
    } else {
        Err(if silent_status == 0 {
            HelloError::Unavailable
        } else {
            error(silent_status)
        })
    }
}
pub fn protect(name: &str, secret: &[u8; 32], parent: isize) -> Result<Vec<u8>> {
    let provider = provider()?;
    let name = key_name(name)?;
    let mut raw = 0;
    // SAFETY: provider/name are live; no overwrite flag is used, so existing keys are preserved.
    check(unsafe {
        NCryptCreatePersistedKey(
            provider.0,
            &mut raw,
            NCRYPT_RSA_ALGORITHM,
            name.as_ptr(),
            0,
            0,
        )
    })?;
    let mut key = Handle(raw);
    let result = (|| {
        set(&key, NCRYPT_LENGTH_PROPERTY, &2048_u32.to_ne_bytes())?;
        set(
            &key,
            NCRYPT_KEY_USAGE_PROPERTY,
            &NCRYPT_ALLOW_DECRYPT_FLAG.to_ne_bytes(),
        )?;
        set(&key, NCRYPT_EXPORT_POLICY_PROPERTY, &0_u32.to_ne_bytes())?;
        set(&key, CACHE_TYPE, &AUTH_MANDATORY.to_ne_bytes())?;
        context(&key, parent)?;
        // SAFETY: this is the newly configured, unfinalized Passport RSA key.
        check(unsafe { NCryptFinalizeKey(key.0, 0) })?;
        mandatory_gesture(&key)?;
        let mut output = vec![0; 256];
        let mut size = 0;
        // SAFETY: fixed buffers match RSA-2048 output and the 32-byte input. Encryption
        // uses the public half only; enrollment separately verifies Hello decryption.
        check(unsafe {
            NCryptEncrypt(
                key.0,
                secret.as_ptr(),
                32,
                null(),
                output.as_mut_ptr(),
                256,
                &mut size,
                NCRYPT_PAD_PKCS1_FLAG,
            )
        })?;
        if size != 256 {
            return Err(HelloError::Data);
        }
        Ok(output)
    })();
    if result.is_err() {
        // SAFETY: only the key created by this invocation is removed. Success consumes it.
        if unsafe { NCryptDeleteKey(key.0, NCRYPT_SILENT_FLAG) } == 0 {
            key.0 = 0;
        }
    }
    result
}
pub fn unprotect(name: &str, protected: &[u8], parent: isize) -> Result<Zeroizing<[u8; 32]>> {
    if protected.len() != 256 {
        return Err(HelloError::Data);
    }
    let provider = provider()?;
    let key = open(&provider, name)?;
    context(&key, parent)?;
    mandatory_gesture(&key)?;
    let mut output = Zeroizing::new([0_u8; 256]);
    let mut size = 0;
    // SAFETY: bounded buffers stay alive for both calls. The silent probe must refuse
    // private-key use; a provider that permits cached/gesture-free use is rejected.
    let silent = unsafe {
        NCryptDecrypt(
            key.0,
            protected.as_ptr(),
            256,
            null(),
            output.as_mut_ptr(),
            256,
            &mut size,
            NCRYPT_PAD_PKCS1_FLAG | NCRYPT_SILENT_FLAG,
        )
    };
    require_interaction(silent)?;
    // SAFETY: same live handle and buffers; omitting SILENT permits the required native Hello UI.
    check(unsafe {
        NCryptDecrypt(
            key.0,
            protected.as_ptr(),
            256,
            null(),
            output.as_mut_ptr(),
            256,
            &mut size,
            NCRYPT_PAD_PKCS1_FLAG,
        )
    })?;
    if size != 32 {
        return Err(HelloError::Data);
    }
    let mut secret = Zeroizing::new([0; 32]);
    secret.copy_from_slice(&output[..32]);
    Ok(secret)
}
pub fn exists(name: &str) -> Result<bool> {
    let provider = provider()?;
    match open(&provider, name) {
        Ok(key) => {
            check_policy(&key)?;
            Ok(true)
        }
        Err(HelloError::KeyMissing) => Ok(false),
        Err(error) => Err(error),
    }
}
pub fn delete(name: &str) -> Result<()> {
    let provider = provider()?;
    let mut key = match open(&provider, name) {
        Ok(key) => key,
        Err(HelloError::KeyMissing) => return Ok(()),
        Err(error) => return Err(error),
    };
    // SAFETY: the validated, app-scoped name selected this owned handle. Success consumes it.
    check(unsafe { NCryptDeleteKey(key.0, NCRYPT_SILENT_FLAG) })?;
    key.0 = 0;
    Ok(())
}
fn current_sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = null_mut();
    // SAFETY: the process pseudo-handle is valid; the owned token is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(HelloError::Unavailable);
    }
    let result = (|| {
        let mut size = 0;
        // SAFETY: the initial query only determines the bounded buffer size.
        unsafe {
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        }
        if size == 0 || size > 65536 {
            return Err(HelloError::Unavailable);
        }
        let mut buffer = vec![0_usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: aligned storage spans size bytes; token and SID remain live until copied.
        unsafe {
            if GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(HelloError::Unavailable);
            }
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
                return Err(HelloError::Unavailable);
            }
            let mut length = 0;
            while length < 256 && *sid.add(length) != 0 {
                length += 1;
            }
            let result = if length < 256 {
                String::from_utf16(std::slice::from_raw_parts(sid, length))
                    .map_err(|_| HelloError::Unavailable)
            } else {
                Err(HelloError::Unavailable)
            };
            LocalFree(sid.cast());
            result
        }
    })();
    // SAFETY: token is the owned handle obtained above, closed exactly once.
    unsafe {
        CloseHandle(token);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_foreign_names_and_maps_errors_without_details() {
        for name in [
            "",
            "Capstan.Hello.abc",
            "Other.key",
            "Capstan.Hello.00000000-0000-4000-8000-000000000000/other",
            "Capstan.Hello.00000000-0000-4000-8000-00000000000\0",
        ] {
            assert!(!valid_name(name));
            assert_eq!(key_name(name).unwrap_err(), HelloError::Data);
        }
        assert!(valid_name(
            "Capstan.Hello.00000000-0000-4000-8000-000000000000"
        ));
        assert_eq!(error(0x80090036_u32 as i32), HelloError::Canceled);
        assert_eq!(error(0x80090016_u32 as i32), HelloError::KeyMissing);
        assert_eq!(
            unprotect("foreign", &[0; 257], 0).unwrap_err(),
            HelloError::Data
        );
    }
    #[test]
    #[ignore = "requires a real enrolled Windows Hello device and interactive PIN/biometric consent"]
    fn real_passport_key_requires_consent_and_roundtrips() {
        let mut random = [0; 16];
        // SAFETY: the fixed writable buffer is live and system-preferred RNG needs no handle.
        assert_eq!(
            unsafe {
                BCryptGenRandom(
                    null_mut(),
                    random.as_mut_ptr(),
                    16,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            },
            0
        );
        let encoded: String = random.iter().map(|value| format!("{value:02x}")).collect();
        let name = format!(
            "Capstan.Hello.{}-{}-{}-{}-{}",
            &encoded[..8],
            &encoded[8..12],
            &encoded[12..16],
            &encoded[16..20],
            &encoded[20..]
        );
        let mut secret = Zeroizing::new([0; 32]);
        // SAFETY: same RNG contract, with an exact initialized output buffer.
        assert_eq!(
            unsafe {
                BCryptGenRandom(
                    null_mut(),
                    secret.as_mut_ptr(),
                    32,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            },
            0
        );
        let protected = protect(&name, &secret, 0).unwrap();
        let result = std::panic::catch_unwind(|| {
            assert_eq!(*unprotect(&name, &protected, 0).unwrap(), *secret);
            assert_eq!(*unprotect(&name, &protected, 0).unwrap(), *secret);
        });
        delete(&name).unwrap();
        assert!(!exists(&name).unwrap());
        result.unwrap()
    }

    #[test]
    fn gesture_free_private_key_use_is_never_accepted() {
        assert_eq!(require_interaction(0), Err(HelloError::Unavailable));
        assert_eq!(require_interaction(SILENT_CONTEXT), Ok(()));
        assert_eq!(
            require_interaction(0x80090036_u32 as i32),
            Err(HelloError::Canceled)
        );
        assert_eq!(
            require_interaction(0x80090005_u32 as i32),
            Err(HelloError::Data)
        );
    }
}
