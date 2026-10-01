use argon2::{
    Argon2, PasswordHash, PasswordHasher as _, PasswordVerifier as _, password_hash::SaltString,
};
use aries_core::auth::{AuthError, PasswordHasher};

#[derive(Debug, Default)]
pub struct Argon2PasswordHasher;

impl PasswordHasher for Argon2PasswordHasher {
    fn hash(&self, password: &str) -> Result<String, AuthError> {
        let mut salt_bytes = [0_u8; 16];
        getrandom::fill(&mut salt_bytes).map_err(|_| AuthError::StoreUnavailable)?;
        let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| AuthError::StoreUnavailable)?;

        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| AuthError::StoreUnavailable)
    }

    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError> {
        // Legacy Aries（Go bcrypt）迁移来的 Hash 前缀为 `$2a$`/`$2y$`，
        // 归一为 `$2b$` 后用 bcrypt 验证；其余按 Argon2id 处理。
        if password_hash.starts_with("$2") {
            let normalized = match &password_hash[..4.min(password_hash.len())] {
                "$2a$" | "$2y$" => format!("$2b${}", &password_hash[4..]),
                _ => password_hash.to_owned(),
            };
            return Ok(bcrypt::verify(password.as_bytes(), &normalized).unwrap_or(false));
        }
        let Ok(parsed) = PasswordHash::new(password_hash) else {
            // 无法识别的 Hash 格式按验证失败处理，而不是内部错误。
            return Ok(false);
        };
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    fn needs_rehash(&self, password_hash: &str) -> bool {
        password_hash.starts_with("$2")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_salted_and_verifiable() {
        let hasher = Argon2PasswordHasher;
        let first = hasher.hash("reliable-pass-2026").unwrap();
        let second = hasher.hash("reliable-pass-2026").unwrap();

        assert_ne!(first, second);
        assert!(hasher.verify("reliable-pass-2026", &first).unwrap());
        assert!(!hasher.verify("wrong-pass-2026", &first).unwrap());
        assert!(!hasher.needs_rehash(&first));
    }

    #[test]
    fn legacy_bcrypt_hashes_verify_and_are_flagged_for_rehash() {
        let hasher = Argon2PasswordHasher;
        // Go `bcrypt.GenerateFromPassword` 产出 `$2a$` 前缀。
        let hash_2a = bcrypt::hash("legacy-pass-2026", 4)
            .unwrap()
            .replacen("$2b$", "$2a$", 1);
        let hash_2y = hash_2a.replacen("$2a$", "$2y$", 1);

        for hash in [&hash_2a, &hash_2y] {
            assert!(hasher.verify("legacy-pass-2026", hash).unwrap());
            assert!(!hasher.verify("wrong-pass-2026", hash).unwrap());
            assert!(hasher.needs_rehash(hash));
        }
        assert!(!hasher.verify("anything", "$2x$not-a-real-hash").unwrap());
    }

    #[test]
    fn unknown_hash_formats_fail_closed() {
        let hasher = Argon2PasswordHasher;
        assert!(!hasher.verify("reliable-pass-2026", "plain-text").unwrap());
        assert!(!hasher.needs_rehash("plain-text"));
    }
}
