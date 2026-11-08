//! The canonical state's version, computed from its layout: a hash of the
//! code that writes and reads it, so it changes when the layout does and
//! is never counted by hand (docs/interpreter.md, "Canonical state").
//!
//! The hash covers the text of `world.rs` between its two layout marks
//! (`World::canonical` and `World::from_canonical`), with whitespace and
//! `//` comments left out, so rewording a comment or reformatting changes
//! nothing and any change to a written or read field changes the version.
//! Two lanes that change the layout get different versions, and a merge of
//! both gets a third, with nothing to number.

const SOURCE: &str = include_str!("world.rs");
const BEGIN: &[u8] = b"// state layout: begin";
const END: &[u8] = b"// state layout: end";

const fn find(hay: &[u8], needle: &[u8], from: usize) -> usize {
    let mut i = from;
    while i + needle.len() <= hay.len() {
        let mut j = 0;
        while j < needle.len() && hay[i + j] == needle[j] {
            j += 1;
        }
        if j == needle.len() {
            return i;
        }
        i += 1;
    }
    panic!("world.rs has no state layout marks");
}

/// FNV-1a over `src[begin..end]` without whitespace and `//` comments.
pub const fn version_of(src: &[u8]) -> u32 {
    let begin = find(src, BEGIN, 0) + BEGIN.len();
    let end = find(src, END, begin);
    let mut h: u32 = 0x811c_9dc5;
    let mut i = begin;
    while i < end {
        let c = src[i];
        if c == b'/' && i + 1 < end && src[i + 1] == b'/' {
            while i < end && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if !(c == b' ' || c == b'\t' || c == b'\n' || c == b'\r') {
            h ^= c as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        i += 1;
    }
    h
}

/// The canonical state's version for this build.
pub const VERSION: u32 = version_of(SOURCE.as_bytes());

#[cfg(test)]
mod tests {
    use super::version_of;

    fn src(body: &str) -> String {
        format!("x\n// state layout: begin\n{body}\n// state layout: end\ny")
    }

    #[test]
    fn comments_and_spacing_dont_change_it_and_code_does() {
        let a = version_of(src("out.push(b.band);").as_bytes());
        let b = version_of(src("  out.push( b.band ); // the band\n").as_bytes());
        let c = version_of(src("out.push(b.band);out.push(b.flag);").as_bytes());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn it_is_not_a_hand_counted_number() {
        // The last hand-counted version was 18.
        let v = super::version_of(super::SOURCE.as_bytes());
        assert_eq!(v, super::VERSION);
        assert!(v > 18, "{v}");
    }
}
