//! Minimal LZ4 *block* decoder (no frame format), used by `Assets.dat`.

use crate::error::{Error, Result};

/// Decodes one LZ4 block, appending to `out`. `max_out` bounds the output
/// produced by this block (guards against corrupt data).
pub(crate) fn decompress_block(src: &[u8], out: &mut Vec<u8>, max_out: usize) -> Result<()> {
    let start = out.len();
    let err = |m: &str| Error::decode("LZ4 block", m.to_string());
    let mut i = 0usize;
    let read_len = |i: &mut usize, mut n: usize| -> Result<usize> {
        if n == 15 {
            loop {
                let b = *src.get(*i).ok_or_else(|| err("truncated length"))?;
                *i += 1;
                n += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        Ok(n)
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let lit = read_len(&mut i, (token >> 4) as usize)?;
        let lit_end = i
            .checked_add(lit)
            .filter(|&e| e <= src.len())
            .ok_or_else(|| err("literal run past end of input"))?;
        out.extend_from_slice(&src[i..lit_end]);
        i = lit_end;
        if i == src.len() {
            break; // last sequence has literals only
        }
        if i + 2 > src.len() {
            return Err(err("truncated match offset"));
        }
        let offset = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        let mlen = read_len(&mut i, (token & 15) as usize)? + 4;
        if offset == 0 || offset > out.len() - start {
            return Err(err("match offset out of range"));
        }
        if out.len() - start + mlen > max_out {
            return Err(err("output larger than expected"));
        }
        let from = out.len() - offset;
        if offset >= mlen {
            out.extend_from_within(from..from + mlen);
        } else {
            for k in 0..mlen {
                let b = out[from + k];
                out.push(b);
            }
        }
    }
    if out.len() - start > max_out {
        return Err(err("output larger than expected"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_overlapping_match() {
        // token: 1 literal, match nibble 15 -> extra length bytes.
        // "a" then copy offset 1, length 15+3+4 = 22 => 23 x 'a', then literal "b".
        let src = [0x1F, b'a', 1, 0, 3, 0x10, b'b'];
        let mut out = Vec::new();
        decompress_block(&src, &mut out, 100).unwrap();
        assert_eq!(out.len(), 24);
        assert!(out[..23].iter().all(|&b| b == b'a'));
        assert_eq!(out[23], b'b');
    }

    #[test]
    fn rejects_bad_offset() {
        let src = [0x10, b'a', 5, 0];
        let mut out = Vec::new();
        assert!(decompress_block(&src, &mut out, 100).is_err());
    }
}
