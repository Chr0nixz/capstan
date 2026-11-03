//! Capstan RSA adapter: private operations are performed by AWS-LC.

use super::{RsaKeypair, RsaPrivateKey};
use crate::{Error, Mpint, Result, public::RsaPublicKey};
use alloc::vec::Vec;
use aws_lc_rs::{
    encoding::{AsDer, Pkcs8V1Der},
    rsa::{KeyPair, KeyPairComponents, KeySize, PublicKeyComponents},
};
use crypto_bigint::{BoxedUint, NonZero};
use zeroize::Zeroizing;

pub(super) fn import_key(public: &RsaPublicKey, private: &RsaPrivateKey) -> Result<KeyPair> {
    let n = public.n().as_positive_bytes().ok_or(Error::Crypto)?;
    let e = public.e().as_positive_bytes().ok_or(Error::Crypto)?;
    // Bound import work by the public modulus and use that same public width
    // for all private values. These reductions only derive OpenSSH's missing
    // CRT exponents during key import, before any network authentication.
    if !(256..=1024).contains(&n.len()) {
        return Err(Error::Crypto);
    }
    let precision = u32::try_from(n.len() * 8).map_err(|_| Error::Crypto)?;
    let d_bytes = private.d().as_positive_bytes().ok_or(Error::Crypto)?;
    let p_bytes = private.p().as_positive_bytes().ok_or(Error::Crypto)?;
    let q_bytes = private.q().as_positive_bytes().ok_or(Error::Crypto)?;
    let d =
        Zeroizing::new(BoxedUint::from_be_slice(d_bytes, precision).map_err(|_| Error::Crypto)?);
    let p =
        Zeroizing::new(BoxedUint::from_be_slice(p_bytes, precision).map_err(|_| Error::Crypto)?);
    let q =
        Zeroizing::new(BoxedUint::from_be_slice(q_bytes, precision).map_err(|_| Error::Crypto)?);
    let one = BoxedUint::one_with_precision(precision);
    let pm1 = Zeroizing::new(
        NonZero::new(p.wrapping_sub(&one))
            .into_option()
            .ok_or(Error::Crypto)?,
    );
    let qm1 = Zeroizing::new(
        NonZero::new(q.wrapping_sub(&one))
            .into_option()
            .ok_or(Error::Crypto)?,
    );
    let dp = Zeroizing::new(d.rem(&*pm1));
    let dq = Zeroizing::new(d.rem(&*qm1));
    let dp_bytes = Zeroizing::new(dp.to_be_bytes());
    let dq_bytes = Zeroizing::new(dq.to_be_bytes());
    KeyPair::from_components(&KeyPairComponents {
        public_key: PublicKeyComponents { n, e },
        d: d_bytes,
        p: p_bytes,
        q: q_bytes,
        dP: dp_bytes.as_ref(),
        dQ: dq_bytes.as_ref(),
        qInv: private.iqmp().as_positive_bytes().ok_or(Error::Crypto)?,
    })
    .map_err(|_| Error::Crypto)
}

pub(super) fn decode_pkcs1(bytes: &[u8]) -> Result<RsaKeypair> {
    let key = pkcs1::RsaPrivateKey::try_from(bytes).map_err(|_| Error::Crypto)?;
    if key.other_prime_infos.is_some() {
        return Err(Error::Crypto);
    }
    // Validate the original CRT parameters as well as the normalized OpenSSH
    // representation. Never silently repair inconsistent imported key material.
    KeyPair::from_der(bytes).map_err(|_| Error::Crypto)?;
    RsaKeypair::new(
        RsaPublicKey::new(
            Mpint::from_positive_bytes(key.public_exponent.as_bytes()),
            Mpint::from_positive_bytes(key.modulus.as_bytes()),
        )?,
        RsaPrivateKey::new(
            Mpint::from_positive_bytes(key.private_exponent.as_bytes()),
            Mpint::from_positive_bytes(key.coefficient.as_bytes()),
            Mpint::from_positive_bytes(key.prime1.as_bytes()),
            Mpint::from_positive_bytes(key.prime2.as_bytes()),
        )?,
    )
}

pub(super) fn encode_pkcs8(key: &KeyPair) -> Result<Zeroizing<Vec<u8>>> {
    let der: Pkcs8V1Der<'static> = key.as_der().map_err(|_| Error::Crypto)?;
    Ok(Zeroizing::new(der.as_ref().to_vec()))
}

pub(super) fn generate(bits: usize) -> Result<RsaKeypair> {
    let size = match bits {
        2048 => KeySize::Rsa2048,
        3072 => KeySize::Rsa3072,
        4096 => KeySize::Rsa4096,
        8192 => KeySize::Rsa8192,
        _ => return Err(Error::Crypto),
    };
    let key = KeyPair::generate(size).map_err(|_| Error::Crypto)?;
    let der = encode_pkcs8(&key)?;
    let info = pkcs8::PrivateKeyInfoRef::try_from(der.as_slice()).map_err(|_| Error::Crypto)?;
    decode_pkcs1(info.private_key.as_bytes())
}
