//! Token 生成、SHA-256 摘要与常量时间比较等密码学小工具。
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::http::error::ApiError;

/// 生成密码学安全随机 Token（URL-Safe Base64）。
pub fn generate_token() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        tracing::error!(error = %error, "failed to generate security token");
        ApiError::internal()
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Token 的 SHA-256 摘要：Session / Reset Token 落库只存该摘要。
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// 常量时间比较，防时序侧信道泄露匹配前缀。
pub fn secrets_equal(left: &str, right: &str) -> bool {
    let left_hash = Sha256::digest(left.as_bytes());
    let right_hash = Sha256::digest(right.as_bytes());
    left_hash.ct_eq(&right_hash).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_random_and_url_safe() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();

        assert_ne!(first, second);
        assert!(!first.contains(['+', '/', '=']));
        assert_eq!(hash_token(&first).len(), 32);
    }
}
