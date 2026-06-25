//! Constant-time comparison helpers for secrets.
//!
//! V-011 fix: the API-key guard compared the supplied key with `==`, which is
//! short-circuiting and byte-wise early-exit — a timing side-channel that
//! leaks the matching prefix + correct length. Over the network the RTT
//! jitter dwarfs the nanosecond-level difference, so this is a low-severity
//! finding, but secrets should never rely on a non-constant-time compare. We
//! route both the length check and the byte compare through `subtle`'s
//! `ConstantTimeEq`, which runs in time independent of the content.
//!
//! The length guard is important: `ConstantTimeEq` on slices of *different*
//! length still returns `false` correctly, but to avoid even a length-revealing
//! compare on the hot path we normalize to "mismatch" up front when lengths
//! differ (the public length of the *expected* key is not itself secret in this
//! app — it's a configured shared key — so a length mismatch is a cheap reject).

use subtle::ConstantTimeEq;

/// Compare two secret byte slices in constant time. Returns `true` iff they
/// are byte-equal AND of equal length.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Different lengths cannot be equal; reject without a content compare.
    if a.len() != b.len() {
        return false;
    }
    // Constant-time compare over the (equal-length) content.
    bool::from(a.ct_eq(b))
}

/// Compare two secret strings in constant time (UTF-8 bytes).
pub fn secret_str_eq(a: &str, b: &str) -> bool {
    constant_time_eq(a.as_bytes(), b.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_strings_match() {
        assert!(secret_str_eq("hunter2", "hunter2"));
    }

    #[test]
    fn different_strings_dont_match() {
        assert!(!secret_str_eq("hunter2", "hunter3"));
    }

    #[test]
    fn different_lengths_dont_match() {
        assert!(!secret_str_eq("a", "ab"));
        assert!(!secret_str_eq("long-secret-value", "short"));
    }

    #[test]
    fn empty_vs_nonempty_dont_match() {
        assert!(!secret_str_eq("", "x"));
        assert!(secret_str_eq("", ""));
    }
}
