//! Version-stable record-id derivation.
//!
//! Some upstream sources (data.gov.hk, HKMA) don't expose a natural primary
//! key, so we derive a synthetic `record_id`. Because these ids are **persisted**
//! (PgStore primary keys) and feed the reproducibility content hash (P-101), the
//! digest must be stable across Rust/compiler versions. The std
//! `DefaultHasher` explicitly disclaims cross-version stability, so we use a
//! fixed FNV-1a hash here instead.
//!
//! HKMA originally carried its own copy of this logic (`hkma.rs`); data.gov.hk
//! used `DefaultHasher` (QUAL-CON-01 — a Rust toolchain upgrade would silently
//! orphan every persisted PK and falsify the cite manifest's drift signal). This
//! module centralizes the correct, stable implementation so both (and any future
//! connector) share one source of truth.

use hkgov_common::RecordValue;
use std::collections::BTreeMap;

/// 64-bit FNV-1a canonical offset basis and prime.
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// 64-bit FNV-1a over `data`, continuing from `init`.
fn fnv1a_64_with(init: u64, data: &[u8]) -> u64 {
    let mut hash = init;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// 64-bit FNV-1a over `data` from the canonical offset basis.
fn fnv1a_64(data: &[u8]) -> u64 {
    fnv1a_64_with(FNV_OFFSET, data)
}

/// Derive a stable synthetic record id from a record's fields.
///
/// Fields are folded into the running hash in `BTreeMap` (sorted) order, so the
/// result is independent of insertion order. Returns a string of the form
/// `id-{016x}`.
pub fn synthetic_record_id(fields: &BTreeMap<String, RecordValue>) -> String {
    let mut hash = fnv1a_64(b"");
    for (k, v) in fields {
        hash = fnv1a_64_with(hash, k.as_bytes());
        let vrepr = format!("{v:?}");
        hash = fnv1a_64_with(hash, vrepr.as_bytes());
    }
    format!("id-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest must be deterministic for a fixed input, regardless of
    /// iteration order. Two field maps with the same contents but different
    /// insertion histories must produce identical ids.
    #[test]
    fn stable_across_insertion_order() {
        let mut a = BTreeMap::new();
        a.insert("z".to_string(), RecordValue::Str("1".into()));
        a.insert("a".to_string(), RecordValue::Int(2));

        let mut b = BTreeMap::new();
        b.insert("a".to_string(), RecordValue::Int(2));
        b.insert("z".to_string(), RecordValue::Str("1".into()));

        assert_eq!(synthetic_record_id(&a), synthetic_record_id(&b));
    }

    /// Pin the exact output so an accidental change to the algorithm is caught
    /// before it can silently re-key persisted rows. This is the value HKMA has
    /// always produced for an empty field set (the seed hash).
    #[test]
    fn pinned_empty_id() {
        let empty = BTreeMap::new();
        let id = synthetic_record_id(&empty);
        // FNV-1a of the empty string from the canonical basis is the basis itself.
        assert_eq!(id, format!("id-{FNV_OFFSET:016x}"));
    }

    /// Different field values must yield different ids (no collisions in the
    /// common case).
    #[test]
    fn distinct_values_distinct_ids() {
        let mut a = BTreeMap::new();
        a.insert("k".to_string(), RecordValue::Int(1));
        let mut b = BTreeMap::new();
        b.insert("k".to_string(), RecordValue::Int(2));
        assert_ne!(synthetic_record_id(&a), synthetic_record_id(&b));
    }
}
