//! SSH signatures (`ssh-keygen -Y sign`, the SSHSIG format) with ed25519
//! keys, checked in pure Rust so `gp` and the browser run the same code.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256, Sha512};

/// The namespace every package signature is made in.
pub const NAMESPACE: &str = "game-printer-package";

const MAGIC: &[u8] = b"SSHSIG";
const KEY_TYPE: &[u8] = b"ssh-ed25519";

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn string(&mut self) -> Result<&'a [u8], String> {
        if self.0.len() < 4 {
            return Err("the signature ends early".into());
        }
        let n = u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]) as usize;
        let rest = &self.0[4..];
        if rest.len() < n {
            return Err("the signature ends early".into());
        }
        self.0 = &rest[n..];
        Ok(&rest[..n])
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend((s.len() as u32).to_be_bytes());
    out.extend(s);
}

/// The 32 key bytes of a key blob, refusing any key type but ed25519.
fn key_bytes(blob: &[u8]) -> Result<[u8; 32], String> {
    let mut r = Reader(blob);
    if r.string()? != KEY_TYPE {
        return Err("only ssh-ed25519 keys are accepted".into());
    }
    r.string()?
        .try_into()
        .map_err(|_| "an ed25519 key is 32 bytes".to_string())
}

/// A public key's blob from its `ssh-ed25519 AAAA... comment` text.
pub fn key_blob(public: &str) -> Result<Vec<u8>, String> {
    let mut parts = public.split_whitespace();
    if parts.next() != Some("ssh-ed25519") {
        return Err(format!("not an ssh-ed25519 key: {public}"));
    }
    let b64 = parts.next().ok_or("a key with no body")?;
    let blob = STANDARD
        .decode(b64)
        .map_err(|e| format!("a key's base64: {e}"))?;
    key_bytes(&blob)?;
    Ok(blob)
}

/// A key's fingerprint, as `ssh-keygen -l` prints it: `SHA256:...`.
pub fn fingerprint(public: &str) -> Result<String, String> {
    let digest = Sha256::digest(key_blob(public)?);
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(digest)))
}

/// Checks an armored SSHSIG over `data`, made in `namespace` by the key
/// `public` (`ssh-ed25519 AAAA...`). The key inside the signature must be
/// that key: the signature can't choose who it claims to be.
pub fn verify(public: &str, namespace: &str, sig: &[u8], data: &[u8]) -> Result<(), String> {
    let want = key_blob(public)?;
    let text = std::str::from_utf8(sig).map_err(|_| "the signature isn't text")?;
    let body: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("-----") && !l.is_empty())
        .collect();
    if !text.contains("-----BEGIN SSH SIGNATURE-----") {
        return Err("not an SSH signature".into());
    }
    let raw = STANDARD
        .decode(body)
        .map_err(|e| format!("the signature's base64: {e}"))?;
    let rest = raw.strip_prefix(MAGIC).ok_or("not an SSH signature")?;
    if rest.len() < 4 || rest[..4] != 1u32.to_be_bytes() {
        return Err("an SSH signature of an unknown version".into());
    }
    let mut r = Reader(&rest[4..]);
    let blob = r.string()?;
    let ns = r.string()?;
    let reserved = r.string()?;
    let alg = r.string()?;
    let inner = r.string()?;
    if blob != want.as_slice() {
        return Err("signed by a different key".into());
    }
    if ns != namespace.as_bytes() {
        return Err(format!(
            "signed in the namespace {:?}, not {namespace:?}",
            String::from_utf8_lossy(ns)
        ));
    }
    let digest: Vec<u8> = match alg {
        b"sha512" => Sha512::digest(data).to_vec(),
        b"sha256" => Sha256::digest(data).to_vec(),
        _ => return Err("an unknown hash in the signature".into()),
    };
    let mut signed = MAGIC.to_vec();
    put_string(&mut signed, ns);
    put_string(&mut signed, reserved);
    put_string(&mut signed, alg);
    put_string(&mut signed, &digest);
    let mut s = Reader(inner);
    if s.string()? != KEY_TYPE {
        return Err("only ssh-ed25519 signatures are accepted".into());
    }
    let bytes: [u8; 64] = s
        .string()?
        .try_into()
        .map_err(|_| "an ed25519 signature is 64 bytes")?;
    let key = VerifyingKey::from_bytes(&key_bytes(&want)?)
        .map_err(|e| format!("a bad public key: {e}"))?;
    key.verify(&signed, &Signature::from_bytes(&bytes))
        .map_err(|_| "the signature doesn't match the data".to_string())
}
