/// A raster format this crate can recognise. Not every variant has a working
/// codec yet — `Gif` and `Apng` currently decode to [`crate::Error::Unsupported`]
/// (WP4/WP5 stubs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    WebP,
    Gif,
    Apng,
}

const RIFF_MAGIC: &[u8; 4] = b"RIFF";
const WEBP_MAGIC: &[u8; 4] = b"WEBP";
/// "RIFF" (4) + a little-endian size (4) + "WEBP" (4).
const RIFF_HEADER_LEN: usize = 12;

const GIF87A_MAGIC: &[u8; 6] = b"GIF87a";
const GIF89A_MAGIC: &[u8; 6] = b"GIF89a";

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// A PNG chunk header is a 4-byte big-endian length followed by a 4-byte type.
const PNG_CHUNK_HEADER_LEN: usize = 8;
const PNG_CHUNK_LENGTH_LEN: usize = 4;
const PNG_CHUNK_TYPE_LEN: usize = 4;
const PNG_CHUNK_CRC_LEN: usize = 4;
const PNG_CHUNK_TYPE_ACTL: &[u8; 4] = b"acTL";
const PNG_CHUNK_TYPE_IDAT: &[u8; 4] = b"IDAT";

/// Identify a format by its bytes, never by a file extension or name — a
/// `.png`-named WebP is still WebP, and a `.webp`-named still PNG is `None`.
///
/// APNG is recognised only when an `acTL` chunk appears before the first
/// `IDAT`; a still PNG (no `acTL`, or one only after `IDAT`) is `None` so it
/// routes to the application's ordinary PNG path. Chunk walking is bounded by
/// the slice length and never panics or overflows on a malformed length.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if is_webp(bytes) {
        return Some(Format::WebP);
    }
    if is_gif(bytes) {
        return Some(Format::Gif);
    }
    if is_apng(bytes) {
        return Some(Format::Apng);
    }
    None
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= RIFF_HEADER_LEN
        && &bytes[0..RIFF_MAGIC.len()] == RIFF_MAGIC
        && &bytes[RIFF_HEADER_LEN - WEBP_MAGIC.len()..RIFF_HEADER_LEN] == WEBP_MAGIC
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.len() >= GIF87A_MAGIC.len()
        && (&bytes[..GIF87A_MAGIC.len()] == GIF87A_MAGIC
            || &bytes[..GIF89A_MAGIC.len()] == GIF89A_MAGIC)
}

fn is_apng(bytes: &[u8]) -> bool {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return false;
    }

    let mut position = PNG_SIGNATURE.len();
    while position + PNG_CHUNK_HEADER_LEN <= bytes.len() {
        let length_bytes: [u8; PNG_CHUNK_LENGTH_LEN] =
            match bytes[position..position + PNG_CHUNK_LENGTH_LEN].try_into() {
                Ok(array) => array,
                Err(_) => return false,
            };
        let data_len = u32::from_be_bytes(length_bytes) as usize;

        let type_start = position + PNG_CHUNK_LENGTH_LEN;
        let type_end = type_start + PNG_CHUNK_TYPE_LEN;
        let chunk_type = &bytes[type_start..type_end];

        if chunk_type == PNG_CHUNK_TYPE_ACTL {
            return true;
        }
        if chunk_type == PNG_CHUNK_TYPE_IDAT {
            return false;
        }

        let next_position = type_end
            .checked_add(data_len)
            .and_then(|end| end.checked_add(PNG_CHUNK_CRC_LEN));
        position = match next_position {
            Some(next) => next,
            None => return false,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_webp_by_content_regardless_of_extension() {
        let mut bytes = Vec::from(*RIFF_MAGIC);
        bytes.extend_from_slice(&[0u8; 4]); // size, unchecked by sniff
        bytes.extend_from_slice(WEBP_MAGIC);
        bytes.extend_from_slice(b"VP8 extra bytes so the buffer isn't tiny");
        assert_eq!(sniff(&bytes), Some(Format::WebP));
    }

    #[test]
    fn sniffs_gif87a_and_gif89a() {
        assert_eq!(sniff(b"GIF87a rest of file"), Some(Format::Gif));
        assert_eq!(sniff(b"GIF89a rest of file"), Some(Format::Gif));
    }

    #[test]
    fn still_png_is_none() {
        let mut bytes = Vec::from(PNG_SIGNATURE);
        // IHDR then straight to IDAT, no acTL.
        push_chunk(&mut bytes, b"IHDR", &[0u8; 13]);
        push_chunk(&mut bytes, b"IDAT", &[0u8; 4]);
        assert_eq!(sniff(&bytes), None);
    }

    #[test]
    fn actl_before_idat_is_apng() {
        let mut bytes = Vec::from(PNG_SIGNATURE);
        push_chunk(&mut bytes, b"IHDR", &[0u8; 13]);
        push_chunk(&mut bytes, b"acTL", &[0u8; 8]);
        push_chunk(&mut bytes, b"IDAT", &[0u8; 4]);
        assert_eq!(sniff(&bytes), Some(Format::Apng));
    }

    #[test]
    fn actl_after_idat_is_not_apng() {
        let mut bytes = Vec::from(PNG_SIGNATURE);
        push_chunk(&mut bytes, b"IHDR", &[0u8; 13]);
        push_chunk(&mut bytes, b"IDAT", &[0u8; 4]);
        push_chunk(&mut bytes, b"acTL", &[0u8; 8]);
        assert_eq!(sniff(&bytes), None);
    }

    #[test]
    fn short_and_garbage_input_is_none() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"xx"), None);
        assert_eq!(sniff(&[0u8; 4]), None);
        assert_eq!(sniff(b"not an image at all, just prose"), None);
    }

    #[test]
    fn oversized_leading_chunk_length_never_panics_and_gives_up_cleanly() {
        let mut bytes = Vec::from(PNG_SIGNATURE);
        // IHDR claims a length that runs off the end of the buffer, so the
        // walk cannot know where the next chunk starts and must give up
        // rather than mis-locate the acTL that follows in the raw bytes.
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        push_chunk(&mut bytes, b"acTL", &[0u8; 8]); // unreachable by design
        assert_eq!(sniff(&bytes), None);
    }

    #[test]
    fn truncated_chunk_header_never_panics() {
        let mut bytes = Vec::from(PNG_SIGNATURE);
        bytes.extend_from_slice(&[0, 0]); // half a length field
        assert_eq!(sniff(&bytes), None);
    }

    fn push_chunk(bytes: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(chunk_type);
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(&[0u8; PNG_CHUNK_CRC_LEN]); // CRC unchecked by sniff
    }
}
