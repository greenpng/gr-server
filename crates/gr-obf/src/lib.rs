//! Compile-time string obfuscation with per-release salt (P0-4).
//!
//! Strings covered by the `obf![]` macro are XOR-encrypted **at compile time**
//! against a key derived from `GR_OBF_SALT` (a fresh random salt is generated
//! by `release/build_and_publish.sh` for every release and injected into each
//! crate by its `build.rs` via `cargo:rustc-env=GR_OBF_KEY=...`).
//!
//! Effects:
//! - `strings` / `grep` over the shipped binary or `.so` no longer shows the
//!   plaintext of covered literals (authorization markers, seal suite names,
//!   license prefixes, OTA error markers, admin secret prompts, ...).
//! - Every release uses a new salt → the encrypted tables + decoded values all
//!   differ, so a patch/script derived from a previous release's binary cannot
//!   be reused (old literals do not appear, old byte offsets do not match).
//!
//! Local/dev builds keep the built-in default salt, so incremental dev builds
//! are stable and tests are deterministic.
//!
//! Usage:
//! ```ignore
//! use gr_obf::obf;
//! let s = obf!("seal master key is empty").s();
//! let hit = obf!("grlic1").eq(tok);          // no-allocation compare
//! println!("{}", obf!("admin bootstrap"));
//! ```
//!
//! Note: the encrypted blob and its key travel together in one `static`, and
//! `option_env!` inside the macro is expanded in the *calling* crate, so the
//! caller's build.rs must inject `GR_OBF_KEY` (see crates/gr-obf/build.rs
//! template — copy it into any crate that uses the macro).

/// ASCII-hex key length injected by build.rs (64 hex chars).
pub const KEY_HEX_LEN: usize = 64;
/// Built-in dev key (stable across local builds; release builds override).
pub const DEFAULT_KEY_HEX: &[u8; KEY_HEX_LEN] =
    b"64657673616c7430303030303030303030303030303030303030303030303030";

/// Normalize an arbitrary source string into the fixed 64-char key (pad or
/// truncate). Const-friendly.
pub const fn key_from_src(src: &[u8]) -> [u8; KEY_HEX_LEN] {
    if src.is_empty() {
        return *DEFAULT_KEY_HEX;
    }
    let mut k = [0u8; KEY_HEX_LEN];
    let mut i = 0;
    while i < KEY_HEX_LEN {
        k[i] = src[i % src.len()];
        i += 1;
    }
    k
}

#[doc(hidden)]
pub const fn default_key() -> [u8; KEY_HEX_LEN] {
    *DEFAULT_KEY_HEX
}

/// Compile-time encrypted blob: `data[i] = plain[i] ^ key[i % 64]`.
pub struct Obf<const N: usize> {
    data: [u8; N],
}

impl<const N: usize> Obf<N> {
    /// Compile-time constructor.
    pub const fn new(plain: &[u8], key: &[u8; KEY_HEX_LEN]) -> Self {
        let mut data = [0u8; N];
        let mut i = 0;
        while i < N {
            data[i] = plain[i] ^ key[i % KEY_HEX_LEN];
            i += 1;
        }
        Self { data }
    }

    /// Decrypt into a `String` (allocates; avoid in hot loops).
    #[inline]
    pub fn decode(&self, key: &[u8; KEY_HEX_LEN]) -> String {
        let mut out = Vec::with_capacity(N);
        for i in 0..N {
            out.push(self.data[i] ^ key[i % KEY_HEX_LEN]);
        }
        String::from_utf8(out).unwrap_or_default()
    }

    /// No-allocation compare against a candidate.
    #[inline]
    pub fn eq(&self, key: &[u8; KEY_HEX_LEN], other: &str) -> bool {
        let b = other.as_bytes();
        if b.len() != N {
            return false;
        }
        let mut x = 0u8;
        for i in 0..N {
            x |= self.data[i] ^ key[i % KEY_HEX_LEN] ^ b[i];
        }
        x == 0
    }

    #[inline]
    pub const fn len(&self) -> usize {
        N
    }
}

/// Blob + its key carried together (created by `obf![]`).
pub struct ObfPair<const N: usize> {
    pub blob: Obf<N>,
    pub key: [u8; KEY_HEX_LEN],
}

impl<const N: usize> ObfPair<N> {
    /// Compile-time constructor from plaintext + arbitrary key source.
    pub const fn build(plain: &[u8], key_src: &[u8]) -> Self {
        let key = key_from_src(key_src);
        let blob = Obf::new(plain, &key);
        Self { blob, key }
    }

    #[inline]
    pub fn s(&self) -> String {
        self.blob.decode(&self.key)
    }

    #[inline]
    pub fn eq(&self, other: &str) -> bool {
        self.blob.eq(&self.key, other)
    }

    #[inline]
    pub const fn len(&self) -> usize {
        N
    }
}

/// Borrowed reference to a compile-time `ObfPair`.
pub struct ObfRef<'a, const N: usize>(pub &'a ObfPair<N>);

impl<'a, const N: usize> ObfRef<'a, N> {
    #[inline]
    pub fn s(&self) -> String {
        self.0.s()
    }

    #[inline]
    pub fn eq(&self, other: &str) -> bool {
        self.0.eq(other)
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}

impl<'a, const N: usize> std::fmt::Display for ObfRef<'a, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.s())
    }
}

impl<'a, const N: usize> std::fmt::Debug for ObfRef<'a, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.s())
    }
}

/// Obfuscate a string literal: `obf!("secret marker")`.
///
/// Expands to an `ObfRef<'static, N>` backed by a `static` with the encrypted
/// bytes and its per-release key. Use `.s()` for a `String`, `.eq(x)` for a
/// compare, or `Display`/`Debug` for formatting.
#[macro_export]
macro_rules! obf {
    ($s:literal) => {{
        const KEY_SRC: &[u8] = match option_env!("GR_OBF_KEY") {
            Some(k) => k.as_bytes(),
            None => $crate::DEFAULT_KEY_HEX,
        };
        static PAIR: $crate::ObfPair<{ $s.as_bytes().len() }> =
            $crate::ObfPair::build($s.as_bytes(), KEY_SRC);
        $crate::ObfRef(&PAIR)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_macro() {
        let s = obf!("grlic1-offline_grace");
        assert_eq!(s.s(), "grlic1-offline_grace");
        assert!(s.eq("grlic1-offline_grace"));
        assert!(!s.eq("grlic1-offline_graceX"));
    }

    #[test]
    fn const_direct_with_dev_key() {
        const B: Obf<4> = Obf::new(b"test", DEFAULT_KEY_HEX);
        assert_eq!(B.decode(DEFAULT_KEY_HEX), "test");
        assert!(B.eq(DEFAULT_KEY_HEX, "test"));
        assert_eq!(B.len(), 4);
    }

    #[test]
    fn key_normalization() {
        let k = key_from_src(b"ab");
        assert_eq!(k.len(), 64);
        assert_eq!(k[0], b'a');
        assert_eq!(k[1], b'b');
        assert_eq!(k[2], b'a');
        let src = b"abcdefghij";
        assert_eq!(key_from_src(src), key_from_src(src));
        // empty source degrades to the dev default (never div-by-zero)
        assert_eq!(key_from_src(b""), *DEFAULT_KEY_HEX);
    }

    #[test]
    fn display_and_debug() {
        assert_eq!(format!("{}", obf!("hello")), "hello");
        assert_eq!(format!("{:?}", obf!("hello")), "hello");
    }

    #[test]
    fn empty_string() {
        let s = obf!("");
        assert_eq!(s.s(), "");
        assert!(s.eq(""));
    }
}
