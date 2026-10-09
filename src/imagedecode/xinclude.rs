//! The XInclude screen: refuse, ahead of gdk-pixbuf's loader chain, any content that
//! could make librsvg process an XInclude (CVE-2026-96889, fixed in librsvg 2.63.2).
//!
//! **Why bytes and not a parse.** The flaw is a use-after-free when an SVG's nested
//! `xi:include` (a `data:` URI suffices) redefines an XML entity. The loader chain
//! picks librsvg by CONTENT, so every hand-off of encoded bytes to it — not only a file
//! named `.svg` — is screened here first, and a refusal is final: the bytes never reach
//! the loader. Content the loader would have rendered is never re-examined by anything
//! downstream, so this screen errs towards refusing. A benign SVG refused here costs a
//! placeholder; a hostile one let through can corrupt memory.
//!
//! **What it refuses, and why each rule is there.** librsvg only acts on an
//! `include` element in the XInclude namespace, so refusing every document that NAMES
//! the namespace, under any prefix, closes the direct route. The remaining rules close
//! the ways the name could reach the parser without appearing in the bytes:
//!
//! | Route to a hidden namespace | Rule |
//! |---|---|
//! | `&#88;Include`, `&#x58;`, `&amp;`-style references | references are expanded before the search |
//! | an entity whose value spells the namespace | any `<!ENTITY` refuses |
//! | an internal DTD subset or an external DTD (which could declare entities) | any DOCTYPE with `[`, or naming a DTD outside `www.w3.org`, refuses |
//! | UTF-16 text (BOM or none), or a switch to UTF-16 mid-stream | content holding a NUL is also read as UTF-16, both byte orders, both alignments |
//! | UCS-4 or EBCDIC, which libxml2 detects from the first four bytes | those signatures refuse |
//! | any other declared encoding (UTF-7, ISO-2022, …) that can spell ASCII without ASCII bytes | an `encoding=` outside the ASCII-compatible allow-list refuses |
//! | gzip (`.svgz`), which librsvg inflates itself | inflated and screened in full |
//! | a nested document in a `data:` URI (`<image>`, `<use>`, CSS `url()`), base64 or percent-encoded | decoded and screened recursively |
//! | CSS escapes (`data\3a`) and the URL parser dropping tabs and newlines | both are undone before the search |
//! | case | every search ignores ASCII case |
//!
//! Recursion is bounded in depth and in total bytes inspected; exceeding either
//! refuses. Relative and `file:` references are not followed: the loader is fed a
//! stream with no base URL, and librsvg resolves no non-`data:` reference without one.
//!
//! Pure: no GTK, no I/O, no global state. The decision of WHETHER to screen
//! ([`ENFORCED`]) and what a refusal does are `super`'s.

use std::io::Read;

/// Whether this build screens at all. Every platform except macOS: the macOS bundle
/// is fixed instead by carrying librsvg 2.63.2 (`packaging/macos/bundle.sh`), while
/// Linux decodes through the distribution's librsvg and Windows through the version
/// gvsbuild pins, neither of which this project controls. A `cfg!` rather than a
/// `#[cfg]` so the screen and its tests compile, and are tested, everywhere.
pub(crate) const ENFORCED: bool = cfg!(not(target_os = "macos"));

/// How deep `data:` URIs and gzip layers may nest before the content is refused.
const MAX_DEPTH: usize = 4;

/// The total bytes one screen may inspect across every layer it decodes. A multiple of
/// the local image cap, so an honest `.svgz` with embedded images fits and a gzip bomb
/// or a quadratic tangle of nested `data:` URIs is refused rather than scanned.
const SCAN_BUDGET: usize = 4 * crate::limits::MAX_LOCAL_IMAGE_BYTES as usize;

const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The first four bytes by which libxml2 recognises a UCS-4 or EBCDIC document — the
/// encodings in which ASCII markup carries no ASCII bytes and which this screen does
/// not decode.
const UNREADABLE_SIGNATURES: &[[u8; 4]] = &[
    [0x00, 0x00, 0x00, 0x3C],
    [0x3C, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x3C, 0x00],
    [0x00, 0x3C, 0x00, 0x00],
    [0x00, 0x00, 0xFE, 0xFF],
    [0xFF, 0xFE, 0x00, 0x00],
    [0x00, 0x00, 0xFF, 0xFE],
    [0xFE, 0xFF, 0x00, 0x00],
    [0x4C, 0x6F, 0xA7, 0x94],
];

/// Stands in for every non-ASCII character in the normalised text. Every pattern this
/// screen looks for is ASCII, so nothing is lost by collapsing the rest.
const NON_ASCII: u8 = 0x80;

/// Where an external DTD may live without refusal: the W3C's own, which declare no
/// entity that could spell anything.
const W3C_DTD_PREFIXES: &[&[u8]] = &[b"http://www.w3.org/", b"https://www.w3.org/"];

/// Encodings in which ASCII text is ASCII bytes (or, for UTF-16, is screened as such).
const ALLOWED_ENCODINGS: &[&[u8]] = &[
    b"utf-8",
    b"utf8",
    b"us-ascii",
    b"ascii",
    b"utf-16",
    b"utf-16le",
    b"utf-16be",
    b"ucs-2",
    b"iso-10646-ucs-2",
    b"latin1",
    b"latin-1",
];
const ALLOWED_ENCODING_FAMILIES: &[&[u8]] = &[
    b"iso-8859-",
    b"iso8859-",
    b"iso_8859-",
    b"windows-125",
    b"cp125",
];

/// The longest a `data:` URI header (`image/svg+xml;charset=utf-8;base64`) may be
/// before the text after `data:` is taken not to be a URI at all.
const MAX_DATA_HEADER: usize = 256;

/// Why content was refused. Logged; the user is told only that the image was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// It names the XInclude namespace.
    XInclude,
    /// It declares an entity, which could spell the namespace.
    EntityDeclaration,
    /// Its DOCTYPE has an internal subset or names a non-W3C DTD.
    Doctype,
    /// It is in, or declares, an encoding this screen cannot read.
    Encoding,
    /// A gzip layer or `data:` payload would not decode.
    Undecodable,
    /// Layers nest deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Decoding its layers would exceed [`SCAN_BUDGET`].
    TooLarge,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::XInclude => "it names the XInclude namespace",
            Self::EntityDeclaration => "it declares an XML entity",
            Self::Doctype => "its DOCTYPE has an internal subset or a non-W3C DTD",
            Self::Encoding => "it uses a character encoding the screen cannot read",
            Self::Undecodable => "an embedded gzip or data: layer does not decode",
            Self::TooDeep => "its embedded layers nest too deeply",
            Self::TooLarge => "its embedded layers expand past the screening budget",
        })
    }
}

/// Screen `bytes`. `Ok` when nothing in them can make librsvg process an XInclude.
pub(crate) fn inspect(bytes: &[u8]) -> Result<(), Reason> {
    let mut budget = SCAN_BUDGET;
    inspect_at(bytes, 0, &mut budget)
}

fn inspect_at(bytes: &[u8], depth: usize, budget: &mut usize) -> Result<(), Reason> {
    if depth > MAX_DEPTH {
        return Err(Reason::TooDeep);
    }
    *budget = budget.checked_sub(bytes.len()).ok_or(Reason::TooLarge)?;
    if bytes.starts_with(GZIP_MAGIC) {
        let inflated = gunzip(bytes, *budget)?;
        return inspect_at(&inflated, depth + 1, budget);
    }
    if bytes
        .get(..4)
        .is_some_and(|head| UNREADABLE_SIGNATURES.iter().any(|sig| head == sig))
    {
        return Err(Reason::Encoding);
    }
    let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    inspect_text(
        &normalise(body.iter().map(|&b| char::from(b))),
        depth,
        budget,
    )?;
    // ASCII written as UTF-16 always has a zero byte beside it, so content with none
    // cannot spell anything when read that way.
    if bytes.contains(&0) {
        for offset in [0, 1] {
            let tail = bytes.get(offset..).unwrap_or_default();
            for big_endian in [false, true] {
                let text = normalise(utf16_chars(tail, big_endian));
                inspect_text(&text, depth, budget)?;
            }
        }
    }
    Ok(())
}

fn gunzip(bytes: &[u8], budget: usize) -> Result<Vec<u8>, Reason> {
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .take(budget as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| Reason::Undecodable)?;
    if out.len() > budget {
        return Err(Reason::TooLarge);
    }
    Ok(out)
}

fn utf16_chars(bytes: &[u8], big_endian: bool) -> impl Iterator<Item = char> + '_ {
    let units = bytes.chunks_exact(2).map(move |pair| {
        let pair = [pair[0], pair[1]];
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
}

/// Reduce text to what a parser would see, for searching: ASCII kept, everything else
/// [`NON_ASCII`]; XML character and predefined-entity references expanded; CSS escapes
/// expanded; tabs and newlines dropped, as a URL parser drops them.
fn normalise(chars: impl Iterator<Item = char>) -> Vec<u8> {
    let ascii: Vec<u8> = chars
        .map(|c| if c.is_ascii() { c as u8 } else { NON_ASCII })
        .collect();
    let mut text = expand_xml_references(&ascii);
    text = expand_css_escapes(&text);
    text.retain(|b| !matches!(b, b'\t' | b'\n' | b'\r'));
    text
}

fn code_point_byte(value: u32) -> u8 {
    match char::from_u32(value) {
        Some(c) if c.is_ascii() => c as u8,
        _ => NON_ASCII,
    }
}

fn expand_xml_references(text: &[u8]) -> Vec<u8> {
    const PREDEFINED: &[(&[u8], u8)] = &[
        (b"amp;", b'&'),
        (b"lt;", b'<'),
        (b"gt;", b'>'),
        (b"quot;", b'"'),
        (b"apos;", b'\''),
    ];
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'&' {
            let rest = &text[i + 1..];
            if let Some((byte, used)) = char_reference(rest) {
                out.push(byte);
                i += 1 + used;
                continue;
            }
            if let Some((name, byte)) = PREDEFINED.iter().find(|(name, _)| rest.starts_with(name)) {
                out.push(*byte);
                i += 1 + name.len();
                continue;
            }
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// `#123;` or `#x7B;` at the start of `rest` → the byte it stands for and the bytes
/// consumed.
fn char_reference(rest: &[u8]) -> Option<(u8, usize)> {
    let digits_from = |start: usize, radix: u32| -> Option<(u8, usize)> {
        let len = rest[start..]
            .iter()
            .take_while(|b| char::from(**b).is_digit(radix))
            .count();
        if len == 0 || rest.get(start + len) != Some(&b';') {
            return None;
        }
        let digits = std::str::from_utf8(&rest[start..start + len]).ok()?;
        let byte = u32::from_str_radix(digits, radix).map_or(NON_ASCII, code_point_byte);
        Some((byte, start + len + 1))
    };
    match rest {
        [b'#', b'x' | b'X', ..] => digits_from(2, 16),
        [b'#', ..] => digits_from(1, 10),
        _ => None,
    }
}

/// CSS escapes: `\` + up to six hex digits (+ one optional space), or `\` + any other
/// character, which stands for itself.
fn expand_css_escapes(text: &[u8]) -> Vec<u8> {
    const MAX_HEX: usize = 6;
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'\\' || i + 1 == text.len() {
            out.push(text[i]);
            i += 1;
            continue;
        }
        let rest = &text[i + 1..];
        let hex = rest
            .iter()
            .take(MAX_HEX)
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        if hex == 0 {
            out.push(rest[0]);
            i += 2;
            continue;
        }
        let digits = std::str::from_utf8(&rest[..hex]).unwrap_or("0");
        out.push(u32::from_str_radix(digits, 16).map_or(NON_ASCII, code_point_byte));
        i += 1 + hex;
        if text.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    out
}

fn inspect_text(text: &[u8], depth: usize, budget: &mut usize) -> Result<(), Reason> {
    if find_ci(text, b"xinclude", 0).is_some() {
        return Err(Reason::XInclude);
    }
    if find_ci(text, b"<!entity", 0).is_some() {
        return Err(Reason::EntityDeclaration);
    }
    check_doctypes(text)?;
    check_encodings(text)?;
    let mut from = 0;
    while let Some(at) = find_ci(text, b"data:", from) {
        from = at + 1;
        if let Some(payload) = data_uri_payload(&text[at + b"data:".len()..])? {
            inspect_at(&payload, depth + 1, budget)?;
        }
    }
    Ok(())
}

fn check_doctypes(text: &[u8]) -> Result<(), Reason> {
    let mut from = 0;
    while let Some(at) = find_ci(text, b"<!doctype", from) {
        from = at + 1;
        let mut quote = None;
        let mut literals: Vec<&[u8]> = Vec::new();
        let mut literal_start = 0;
        let mut closed = false;
        for (i, &b) in text.iter().enumerate().skip(at) {
            match quote {
                Some(q) if b == q => {
                    literals.push(&text[literal_start..i]);
                    quote = None;
                }
                Some(_) => {}
                None if b == b'"' || b == b'\'' => {
                    quote = Some(b);
                    literal_start = i + 1;
                }
                None if b == b'[' => return Err(Reason::Doctype),
                None if b == b'>' => {
                    closed = true;
                    break;
                }
                None => {}
            }
        }
        if !closed {
            return Err(Reason::Doctype);
        }
        if let Some(system) = literals.last() {
            if !W3C_DTD_PREFIXES.iter().any(|p| starts_with_ci(system, p)) {
                return Err(Reason::Doctype);
            }
        }
    }
    Ok(())
}

fn check_encodings(text: &[u8]) -> Result<(), Reason> {
    let mut from = 0;
    while let Some(at) = find_ci(text, b"<?xml", from) {
        from = at + 1;
        let after = at + b"<?xml".len();
        // `<?xml-stylesheet …?>` and friends are other processing instructions.
        if !text.get(after).is_some_and(u8::is_ascii_whitespace) {
            continue;
        }
        let end = find_ci(text, b"?>", after).unwrap_or(text.len());
        let decl = &text[after..end];
        let Some(key) = find_ci(decl, b"encoding", 0) else {
            continue;
        };
        let name = declared_value(&decl[key + b"encoding".len()..]).ok_or(Reason::Encoding)?;
        let allowed = ALLOWED_ENCODINGS
            .iter()
            .any(|e| name.eq_ignore_ascii_case(e))
            || ALLOWED_ENCODING_FAMILIES
                .iter()
                .any(|f| starts_with_ci(name, f));
        if !allowed {
            return Err(Reason::Encoding);
        }
    }
    Ok(())
}

/// `= "value"` (whitespace allowed around `=`) → `value`.
fn declared_value(rest: &[u8]) -> Option<&[u8]> {
    let rest = trim_start(rest).strip_prefix(b"=")?;
    let rest = trim_start(rest);
    let (&quote, rest) = rest.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let len = rest.iter().position(|&b| b == quote)?;
    Some(&rest[..len])
}

fn trim_start(bytes: &[u8]) -> &[u8] {
    let skip = bytes.iter().take_while(|b| b.is_ascii_whitespace()).count();
    &bytes[skip..]
}

/// The decoded payload of the `data:` URI whose text follows `data:` in `rest`.
/// `Ok(None)` when it is not a URI (no `,` ends a plausible header). The extent is
/// over-approximated wherever the end is uncertain — screening more text than the URI
/// holds can only refuse more.
fn data_uri_payload(rest: &[u8]) -> Result<Option<Vec<u8>>, Reason> {
    let Some(comma) = rest
        .iter()
        .take(MAX_DATA_HEADER)
        .take_while(|b| !matches!(b, b'"' | b'\''))
        .position(|&b| b == b',')
    else {
        return Ok(None);
    };
    let header = &rest[..comma];
    let body = &rest[comma + 1..];
    if ends_with_ci(trim_end(header), b";base64") {
        let len = body
            .iter()
            .take_while(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'%' | b' ')
            })
            .count();
        let decoded = percent_decode(&body[..len]);
        return base64_decode(&decoded).map(Some).ok_or(Reason::Undecodable);
    }
    let len = body
        .iter()
        .position(|b| matches!(b, b'"' | b'\''))
        .unwrap_or(body.len());
    Ok(Some(percent_decode(&body[..len])))
}

fn trim_end(bytes: &[u8]) -> &[u8] {
    let keep = bytes.len()
        - bytes
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
    &bytes[..keep]
}

fn percent_decode(text: &[u8]) -> Vec<u8> {
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'%' {
            if let (Some(hi), Some(lo)) = (
                text.get(i + 1).copied().and_then(hex),
                text.get(i + 2).copied().and_then(hex),
            ) {
                // Two hex digits always fit a byte.
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// Forgiving base64, as a `data:` URL decoder reads it: ASCII whitespace ignored, up to
/// two `=` of padding, which may be absent. `None` for anything else.
fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    const BITS_PER_SYMBOL: u32 = 6;
    let symbol = |b: u8| -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let mut clean: Vec<u8> = text
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if clean.len().is_multiple_of(4) {
        for _ in 0..2 {
            if clean.last() == Some(&b'=') {
                clean.pop();
            }
        }
    }
    if clean.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in clean {
        acc = (acc << BITS_PER_SYMBOL) | symbol(b)?;
        bits += BITS_PER_SYMBOL;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn starts_with_ci(hay: &[u8], prefix: &[u8]) -> bool {
    hay.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn ends_with_ci(hay: &[u8], suffix: &[u8]) -> bool {
    hay.len() >= suffix.len() && hay[hay.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// The first case-insensitive occurrence of ASCII `needle` in `hay` at or after `from`.
fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let first = needle.first()?.to_ascii_lowercase();
    let last_start = hay.len().checked_sub(needle.len())?;
    (from..=last_start).find(|&i| {
        hay[i].to_ascii_lowercase() == first
            && hay[i..i + needle.len()].eq_ignore_ascii_case(needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The published trigger's shape: an `xi:include` of a `data:` URI.
    const ATTACK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xi="http://www.w3.org/2001/XInclude" width="8" height="8"><xi:include href="data:text/xml,%3C!DOCTYPE%20a%5B%3C!ENTITY%20e%20'x'%3E%5D%3E%3Ca%3E%26e;%3C/a%3E" parse="xml"/></svg>"#;

    /// A plain badge-style SVG with nothing to refuse.
    const BENIGN: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="90" height="20">
  <style>.t { font: 11px sans-serif; fill: url(#g); }</style>
  <linearGradient id="g"><stop offset="0" stop-color="#bbb"/></linearGradient>
  <rect width="90" height="20" rx="3" fill="#555"/>
  <text x="45" y="14" class="t">build &amp; test &#169; &lt;ok&gt;</text>
  <image width="1" height="1" xlink:href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="/>
</svg>"##;

    fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let units = bom
            .then_some(0xFEFF_u16)
            .into_iter()
            .chain(text.encode_utf16());
        for unit in units {
            out.extend(if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        out
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).expect("gzip write");
        enc.finish().expect("gzip finish")
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn refused(bytes: impl AsRef<[u8]>) -> Option<Reason> {
        inspect(bytes.as_ref()).err()
    }

    #[test]
    fn the_published_trigger_is_refused() {
        assert_eq!(refused(ATTACK), Some(Reason::XInclude));
    }

    #[test]
    fn the_namespace_is_refused_under_any_prefix_or_as_the_default() {
        for svg in [
            r#"<svg xmlns:zz="http://www.w3.org/2001/XInclude"><zz:include href="a"/></svg>"#,
            r#"<svg><include xmlns="http://www.w3.org/2001/XInclude" href="a"/></svg>"#,
            r#"<svg xmlns:q='http://www.w3.org/2003/XInclude'/>"#,
            r#"<svg xmlns:q="HTTP://WWW.W3.ORG/2001/XINCLUDE"/>"#,
        ] {
            assert_eq!(refused(svg), Some(Reason::XInclude), "{svg}");
        }
    }

    #[test]
    fn a_namespace_spelled_with_references_or_split_by_newlines_is_refused() {
        for svg in [
            r#"<svg xmlns:a="http://www.w3.org/2001/&#88;Include"/>"#,
            r#"<svg xmlns:a="http://www.w3.org/2001/&#x58;&#x49;nclude"/>"#,
            r#"<svg xmlns:a="http://www.w3.org/2001/XIn&#000099;lude"/>"#,
            "<svg xmlns:a=\"http://www.w3.org/2001/XIn\nclude\"/>",
        ] {
            assert_eq!(refused(svg), Some(Reason::XInclude), "{svg}");
        }
    }

    #[test]
    fn entity_declarations_and_foreign_dtds_are_refused() {
        assert_eq!(
            refused(
                r#"<!DOCTYPE svg [<!ENTITY ns "http://www.w3.org/2001/X&#37;49nclude">]><svg/>"#
            ),
            Some(Reason::EntityDeclaration)
        );
        assert_eq!(
            refused(r#"<!DOCTYPE svg [ <!-- subset --> ]><svg/>"#),
            Some(Reason::Doctype)
        );
        assert_eq!(
            refused(r#"<!DOCTYPE svg SYSTEM "data:application/xml-dtd,x"><svg/>"#),
            Some(Reason::Doctype)
        );
        assert_eq!(
            refused(r#"<!DOCTYPE svg SYSTEM "evil.dtd"><svg/>"#),
            Some(Reason::Doctype)
        );
        assert_eq!(refused(r#"<!DOCTYPE svg"#), Some(Reason::Doctype));
    }

    #[test]
    fn utf16_in_either_byte_order_with_or_without_a_bom_is_read() {
        for big_endian in [false, true] {
            for bom in [false, true] {
                assert_eq!(
                    refused(utf16(ATTACK, big_endian, bom)),
                    Some(Reason::XInclude),
                    "big_endian={big_endian} bom={bom}"
                );
                assert_eq!(refused(utf16(BENIGN, big_endian, bom)), None);
            }
        }
    }

    #[test]
    fn a_mid_stream_switch_to_utf16_at_an_odd_offset_is_read() {
        let mut bytes = b"<?xml version='1.0' encoding='UTF-16'?>x".to_vec();
        bytes.extend(utf16(ATTACK, false, false));
        assert_eq!(refused(bytes), Some(Reason::XInclude));
    }

    #[test]
    fn unreadable_encodings_are_refused() {
        assert_eq!(
            refused([0x4C, 0x6F, 0xA7, 0x94, 0x40]),
            Some(Reason::Encoding)
        );
        assert_eq!(
            refused([0, 0, 0, 0x3C, 0, 0, 0, 0x73]),
            Some(Reason::Encoding)
        );
        for enc in ["UTF-7", "ISO-2022-JP", "IBM037", "UCS-4", "Shift_JIS"] {
            let svg = format!(r#"<?xml version="1.0" encoding="{enc}"?><svg/>"#);
            assert_eq!(refused(&svg), Some(Reason::Encoding), "{enc}");
        }
        assert_eq!(
            refused(r#"<?xml version="1.0" encoding=UTF-7?><svg/>"#),
            Some(Reason::Encoding),
            "an unquoted value is unreadable, so refused"
        );
        for enc in ["utf-8", "ISO-8859-1", "windows-1252", "US-ASCII", "UTF-16"] {
            let svg = format!(r#"<?xml version="1.0" encoding = '{enc}' ?><svg/>"#);
            assert_eq!(refused(&svg), None, "{enc}");
        }
    }

    #[test]
    fn a_gzipped_svgz_is_inflated_and_screened() {
        assert_eq!(refused(gzip(ATTACK.as_bytes())), Some(Reason::XInclude));
        assert_eq!(
            refused(gzip(&gzip(ATTACK.as_bytes()))),
            Some(Reason::XInclude)
        );
        assert_eq!(refused(gzip(BENIGN.as_bytes())), None);
        assert_eq!(
            refused([0x1f, 0x8b, 0x08, 0x00, 0xde, 0xad]),
            Some(Reason::Undecodable)
        );
    }

    #[test]
    fn a_nested_document_in_a_data_uri_is_screened() {
        let base64_svg = format!(
            r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="data:image/svg+xml;base64,{}"/></svg>"#,
            base64(ATTACK.as_bytes())
        );
        assert_eq!(refused(&base64_svg), Some(Reason::XInclude));
        let gz_base64 = format!(
            r#"<svg><use href="data:image/svg+xml;base64,{}#x"/></svg>"#,
            base64(&gzip(ATTACK.as_bytes()))
        );
        assert_eq!(refused(&gz_base64), Some(Reason::XInclude));
        assert_eq!(
            refused(
                r#"<svg><image href="data:image/svg+xml,%3Csvg xmlns:a=%22http://www.w3.org/2001/%58Include%22/%3E"/></svg>"#
            ),
            Some(Reason::XInclude)
        );
        assert_eq!(
            refused(
                r#"<svg><style>rect { fill: url(data\3a image/svg+xml;base64,AAAA!) }</style></svg>"#
            ),
            None,
            "a payload that stops at a non-base64 character decodes the prefix"
        );
        assert_eq!(
            refused(r#"<svg><image href="data:image/svg+xml;base64,QUJD="/></svg>"#),
            Some(Reason::Undecodable),
            "padding in the wrong place does not decode, so refuses"
        );
    }

    #[test]
    fn a_data_scheme_spelled_with_css_escapes_or_references_is_still_found() {
        let hidden = format!(
            r#"<svg><style>rect {{ fill: url(da\74 a:image/svg+xml;base64,{}) }}</style><image href="&#100;ata:image/svg+xml;base64,{}"/></svg>"#,
            base64(ATTACK.as_bytes()),
            base64(ATTACK.as_bytes())
        );
        assert_eq!(refused(&hidden), Some(Reason::XInclude));
    }

    #[test]
    fn nesting_past_the_depth_limit_is_refused() {
        let mut bytes = b"<svg/>".to_vec();
        for _ in 0..=MAX_DEPTH {
            bytes = gzip(&bytes);
        }
        assert_eq!(refused(bytes), Some(Reason::TooDeep));
    }

    #[test]
    fn benign_content_passes() {
        assert_eq!(refused(BENIGN), None);
        assert_eq!(refused(format!("\u{FEFF}{BENIGN}")), None);
        assert_eq!(refused(r#"<?xml-stylesheet href="s.css"?><svg/>"#), None);
        assert_eq!(refused(r#"<svg><text>metadata: none</text></svg>"#), None);
        // A still PNG's real header and IDAT, which carry zero bytes.
        let png = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/wide.png"
        ));
        assert_eq!(refused(png), None);
        let svg = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/diagram.svg"
        ));
        assert_eq!(refused(svg), None);
    }

    /// The manual-test fixtures (MANUAL-TEST 2.23c) are what this screen says they
    /// are, so a placeholder seen there is the screen's doing.
    #[test]
    fn the_manual_test_fixtures_are_refused_except_the_plain_one() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xinclude");
        for name in [
            "prefixed.svg",
            "hidden.svg",
            "utf16.svg",
            "compressed.svgz",
            "nested.svg",
        ] {
            let bytes = std::fs::read(dir.join(name)).expect("fixture");
            assert_eq!(refused(bytes), Some(Reason::XInclude), "{name}");
        }
        let plain = std::fs::read(dir.join("plain.svg")).expect("fixture");
        assert_eq!(refused(plain), None);
    }

    #[test]
    fn the_helpers_decode_what_they_claim() {
        assert_eq!(base64_decode(b"aGVs bG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode(b"aGVsbG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode(b"a"), None);
        assert_eq!(percent_decode(b"%3Cx%zz%4"), b"<x%zz%4");
        assert_eq!(find_ci(b"abXINCLUDE", b"xinclude", 0), Some(2));
        assert_eq!(find_ci(b"ab", b"xinclude", 0), None);
    }
}
