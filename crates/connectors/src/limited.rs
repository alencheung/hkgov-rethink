//! Size-limited response reads.
//!
//! Every connector reads an upstream body with `resp.text()` / `resp.json()`,
//! which materializes the entire payload into memory before parsing — and then
//! parsing allocates a second (often larger) structure on top. A misbehaving or
//! malicious upstream returning a multi-gigabyte body therefore OOMs the whole
//! process (peak ~3–5× body size), taking down every other source with it
//! (PERF-CON-01).
//!
//! These helpers cap the raw body at a fixed ceiling before parsing, so memory
//! stays bounded regardless of upstream behavior. The cap is generous (64 MiB
//! for data, 1 MiB for error bodies) — the largest legitimate payload we see is
//! HKMA's verified dataset table, well under that.

use hkgov_common::{Error, Result};

/// Default cap on a data (success-body) response. Large enough for the biggest
/// legitimate HKGOV payload we ingest (HKMA's full monetary-statistics table),
/// small enough that a runaway upstream can't OOM the process.
pub const MAX_DATA_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

/// Default cap on an error-body read (we only read errors to surface a
/// diagnostic string; 1 MiB is far more than any sane error page).
pub const MAX_ERROR_BYTES: usize = 1024 * 1024; // 1 MiB

/// Read a response body into a `String`, capped at `max_bytes`.
///
/// Streams the body chunk-by-chunk and aborts with `Error::Upstream` the moment
/// the running total exceeds `max_bytes`, so peak memory is bounded by the cap,
/// not by the upstream. The body is decoded as UTF-8 (lossily), matching the
/// prior `resp.text()` semantics for the non-UTF-8 fragments HK gov feeds
/// occasionally emit.
pub async fn read_text_limited(
    resp: reqwest::Response,
    origin: &'static str,
    max_bytes: usize,
) -> Result<String> {
    use futures_util::stream::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Upstream {
            origin,
            status: 0,
            detail: format!("body read: {e}"),
        })?;
        // Bound check BEFORE extending so we never hold >max_bytes even briefly.
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(Error::Upstream {
                origin,
                status: 0,
                detail: format!(
                    "upstream body exceeded {max_bytes} byte cap — refusing to buffer further (possible runaway upstream)"
                ),
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Read + JSON-decode a capped response body in one step. `decode` turns the
/// capped text into the target type, so callers keep their existing
/// `serde_json::from_str` / shape-specific error mapping.
pub async fn read_text_limited_then<T, F>(
    resp: reqwest::Response,
    origin: &'static str,
    max_bytes: usize,
    decode: F,
) -> Result<T>
where
    F: FnOnce(&str) -> Result<T>,
{
    let text = read_text_limited(resp, origin, max_bytes).await?;
    decode(&text)
}

// Compile-time invariant checks on the caps. `assertions_on_constants` (clippy
// 1.97) flags runtime `assert!` on constants; a const-block makes these a
// build-time check instead, so an edit that shrinks the data cap below a sane
// floor or sets the error cap above the data cap fails to compile.
const _: () = {
    assert!(MAX_DATA_BYTES >= 32 * 1024 * 1024);
    assert!(MAX_DATA_BYTES <= 256 * 1024 * 1024);
    assert!(MAX_ERROR_BYTES < MAX_DATA_BYTES);
};

#[cfg(test)]
mod tests {
    use super::*;
}
