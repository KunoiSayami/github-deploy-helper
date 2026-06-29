use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verifies the GitHub `X-Hub-Signature-256` header value against the request body.
/// The header format is `sha256=<hex>`. Uses constant-time comparison to prevent timing attacks.
pub fn verify(secret: &str, body: &[u8], header_value: &str) -> bool {
    let Some(hex_sig) = header_value.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature() {
        let secret = "mysecret";
        let body = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let header = format!("sha256={}", hex::encode(result));
        assert!(verify(secret, body, &header));
    }

    #[test]
    fn invalid_signature() {
        assert!(!verify("secret", b"body", "sha256=deadbeef"));
    }

    #[test]
    fn missing_prefix() {
        assert!(!verify("secret", b"body", "deadbeef"));
    }
}
