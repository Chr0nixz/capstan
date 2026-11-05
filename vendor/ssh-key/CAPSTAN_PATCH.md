# Capstan ssh-key RSA backend patch

Upstream: ssh-key 0.7.0-rc.10, revision
`c8605550cb31f750c3bf318eea433141af84f2c7` in RustCrypto/SSH (`ssh-key`).
The library source, original manifest and MIT/Apache-2.0 license texts are
retained. Upstream external fixtures and test targets are not vendored; Capstan
generates keys at test runtime. The root Cargo patch selects this copy.

The `rsa` feature uses pinned `aws-lc-rs 1.18.1`, not RustCrypto's `rsa` crate.
AWS-LC performs key generation, private-key consistency validation, RSA-SHA256
and RSA-SHA512 signing, and RSA public verification. The backend is constructed
and validated when the key is decoded, before network authentication, and shared
by cloned keypairs. No RSA decryption API is introduced.

OpenSSH omits CRT exponents. During local import only, those exponents are
derived using crypto-bigint's constant-time remainder operation at the public
modulus width. Temporary private integers and derived byte buffers use
Zeroizing. AWS-LC validates all supplied components, including the original
CRT values for PKCS#1/PKCS#8 input. Unsupported sizes and inconsistent material
return the existing redacted Crypto error.

The local manifest pins crypto-bigint 0.7.9, the lowest non-yanked release
with the RUSTSEC-2026-0387 fix (patched in 0.7.6). Upstream yanked 0.7.6–0.7.8
for a breaking `NonZero<T>` `Mul` bound, fixed in 0.7.9. The 0.7.9 update keeps
the constant-time fixes and adds `invert_odd_mod` and `mul_mod_special`
correctness fixes. This patch update retains the existing RSA backend and
key-import boundaries; the complete resolved dependency graph remains subject
to the release audit.

Generated keys support 2048/3072/4096/8192 bits. Private imports require a public
modulus of 2048–8192 bits. New signatures use SHA-256/SHA-512; legacy SHA-1
verification remains available for host-key interoperability. SHA-1 signing is
rejected. Capstan already selects RSA SHA-2 for private-key authentication.

Capstan tests cover both SHA-2 signature forms, altered-message rejection,
OpenSSH/PKCS#8 import, an in-process SSH server, inconsistent private components
and weak generation requests. The disposable OpenSSH fixture additionally
generates OpenSSH encrypted/unencrypted, PKCS#1, and PKCS#8 encrypted/unencrypted
RSA files for external interoperability checks. These tests validate adapter
behavior and interoperability; they are not a new cryptographic audit.
