use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Context, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub struct Secrets {
    cipher: Aes256Gcm,
    api_key_hmac: [u8; 32],
}

impl Secrets {
    pub fn from_master_key(b64: &str) -> anyhow::Result<Self> {
        let key = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .context("BRAID_MASTER_KEY must be base64")?;
        if key.len() != 32 {
            return Err(anyhow!("BRAID_MASTER_KEY must decode to 32 bytes (generate with: openssl rand -base64 32)"));
        }
        let mut api_key_hmac = [0u8; 32];
        api_key_hmac.copy_from_slice(&Sha256::digest([b"braid-api-key-hmac:".as_slice(), &key].concat()));
        Ok(Self { cipher: Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("bad key"))?, api_key_hmac })
    }

    /// Output layout: 12-byte nonce followed by ciphertext+tag.
    pub fn encrypt(&self, plaintext: &str) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        rand::fill(&mut nonce);
        let ct = self.cipher.encrypt(&Nonce::from(nonce), plaintext.as_bytes()).expect("aes-gcm encrypt");
        [nonce.as_slice(), &ct].concat()
    }

    pub fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> {
        if data.len() < 12 {
            return Err(anyhow!("ciphertext too short"));
        }
        let nonce: [u8; 12] = data[..12].try_into()?;
        let pt = self
            .cipher
            .decrypt(&Nonce::from(nonce), &data[12..])
            .map_err(|_| anyhow!("cannot decrypt secret; was BRAID_MASTER_KEY changed?"))?;
        Ok(String::from_utf8(pt)?)
    }

    pub fn api_key_hash(&self, key: &str) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.api_key_hmac).expect("hmac key");
        mac.update(key.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
}

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(buf.as_mut_slice());
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn sha256(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(Argon2::default().hash_password(password.as_bytes()).map_err(|e| anyhow!("{e}"))?.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

pub fn last4(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// RFC 6238 TOTP (HMAC-SHA1, 30 s, 6 digits). Returns the matching step within ±1 step of `now`.
pub fn totp_match(secret: &[u8], code: &str, now: u64) -> Option<u64> {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let step = now / 30;
    [step.saturating_sub(1), step, step + 1].into_iter().find(|s| totp_at(secret, *s) == code)
}

fn totp_at(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret).expect("hmac key");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let n = (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000;
    format!("{n:06}")
}

pub fn base32(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let v = u64::from_be_bytes([0, 0, 0, buf[0], buf[1], buf[2], buf[3], buf[4]]);
        for i in 0..(chunk.len() * 8).div_ceil(5) {
            out.push(A[((v >> (35 - i * 5)) & 31) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let s = Secrets::from_master_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let ct = s.encrypt("sk-test-1234");
        assert_eq!(s.decrypt(&ct).unwrap(), "sk-test-1234");
        assert_ne!(s.encrypt("x"), s.encrypt("x"));
        let h = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert_eq!(last4("sk-abcdef"), "cdef");
        assert_eq!(s.api_key_hash("k"), s.api_key_hash("k"));
    }

    #[test]
    fn totp_rfc6238_vector() {
        // RFC 6238 appendix B, SHA1, T=59 -> 94287082 (last 6 digits).
        let secret = b"12345678901234567890";
        assert_eq!(totp_at(secret, 59 / 30), "287082");
        assert_eq!(totp_match(secret, "287082", 59), Some(1));
        assert_eq!(totp_match(secret, "287082", 59 + 90), None);
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }
}
