//! [`Secret`] — zeroized-on-drop raw bytes value object.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Opaque secret bytes that are zeroed from memory on drop.
///
/// The only way to access the bytes is via [`as_bytes`](Secret::as_bytes).
/// No `String` or `Display` implementation is provided.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// Wraps raw bytes in a `Secret`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns a slice of the underlying secret bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_bytes_roundtrip() {
        let bytes = b"super-secret".to_vec();
        let s = Secret::new(bytes.clone());
        assert_eq!(s.as_bytes(), bytes.as_slice());
    }

    #[test]
    fn different_bytes_not_equal() {
        let s1 = Secret::new(b"abc".to_vec());
        let s2 = Secret::new(b"xyz".to_vec());
        assert_ne!(s1.as_bytes(), s2.as_bytes());
    }

    // Compile-time check: Secret derives Zeroize + ZeroizeOnDrop.
    // If the derive is missing this function will not compile.
    #[test]
    fn zeroize_on_drop_compiles() {
        fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<Secret>();
    }
}
