use sha2::{Digest, Sha256};

/// Stable short hash: first 16 hex chars of sha256(text).
pub fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha256_prefix() {
        assert_eq!(super::hash("test"), "9f86d081884c7d65");
    }
}
