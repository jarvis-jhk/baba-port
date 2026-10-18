//! The game's executable, read only for the names of the sound effects.
//!
//! `Assets.dat` stores the sound effects by index; the names the game's
//! scripts use for them exist only in the executable, as a string hash
//! table (pointers to names, empty slots in between) next to a parallel
//! array of 32-bit sound indices. The same table is in every build:
//!
//! | Build | Executable | Container |
//! |---|---|---|
//! | Linux | `bin64/Chowdren` | ELF |
//! | Windows | `Baba Is You.exe` | PE32+ |
//! | macOS | `Baba Is You.app/Contents/MacOS/Chowdren` | Mach-O (thin or universal) |
//!
//! The pointers are plain virtual addresses in all three (ELF without PIE,
//! PE at its preferred image base, Mach-O with classic rebase info), so one
//! search works once the file's segments are known: find a pointer to a
//! known name, grow the table of name pointers around it, then find the
//! index array that maps its filled slots onto `0..count`. Details:
//! `docs/formats/assets-dat.md`. The executable is the player's own; the
//! names are recovered at run time and never stored.

/// Container format of an executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExeFormat {
    /// ELF (Linux).
    Elf,
    /// PE / COFF (Windows).
    Pe,
    /// Mach-O, one architecture (macOS).
    MachO,
    /// Universal Mach-O with several architectures (macOS).
    MachOFat,
}

impl ExeFormat {
    /// Detects the container from the first bytes.
    pub fn detect(b: &[u8]) -> Option<ExeFormat> {
        match b.get(..4)? {
            [0x7f, b'E', b'L', b'F'] => Some(ExeFormat::Elf),
            [b'M', b'Z', ..] => Some(ExeFormat::Pe),
            [0xcf, 0xfa, 0xed, 0xfe] | [0xce, 0xfa, 0xed, 0xfe] => Some(ExeFormat::MachO),
            [0xca, 0xfe, 0xba, 0xbe] | [0xca, 0xfe, 0xba, 0xbf] => Some(ExeFormat::MachOFat),
            _ => None,
        }
    }
}

/// A loaded region: virtual address, file offset, bytes present in the file.
#[derive(Clone, Copy, Debug)]
struct Segment {
    va: u64,
    off: u64,
    size: u64,
}

/// A parsed single-architecture image.
struct Image<'a> {
    b: &'a [u8],
    segs: Vec<Segment>,
    /// Pointer size in bytes (4 or 8).
    ptr: usize,
}

fn le16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn le32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn le64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}
fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn be64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

impl<'a> Image<'a> {
    fn parse(b: &'a [u8]) -> Option<Image<'a>> {
        let (segs, ptr) = match ExeFormat::detect(b)? {
            ExeFormat::Elf => elf_segments(b)?,
            ExeFormat::Pe => pe_segments(b)?,
            ExeFormat::MachO => macho_segments(b)?,
            ExeFormat::MachOFat => return None,
        };
        (!segs.is_empty()).then_some(Image { b, segs, ptr })
    }

    fn va_to_off(&self, va: u64) -> Option<usize> {
        self.segs
            .iter()
            .find(|s| va >= s.va && va < s.va + s.size)
            .and_then(|s| usize::try_from(s.off + va - s.va).ok())
    }

    fn off_to_va(&self, off: u64) -> Option<u64> {
        self.segs
            .iter()
            .find(|s| off >= s.off && off < s.off + s.size)
            .map(|s| s.va + off - s.off)
    }

    fn read_ptr(&self, o: usize) -> Option<u64> {
        if self.ptr == 8 {
            le64(self.b, o)
        } else {
            le32(self.b, o).map(u64::from)
        }
    }

    /// The identifier (letters, digits, `_`; possibly empty) at `va`.
    fn ident(&self, va: u64) -> Option<&'a str> {
        let o = self.va_to_off(va)?;
        let tail = self.b.get(o..)?;
        let end = tail.iter().take(64).position(|&c| c == 0)?;
        let s = &tail[..end];
        s.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .then(|| std::str::from_utf8(s).ok())?
    }

    fn sound_names(&self, count: usize) -> Option<Vec<String>> {
        let b = self.b;
        let p = self.ptr;
        let needle = b"\0winnery_fast\0";
        let at = b.windows(needle.len()).position(|w| w == needle)? + 1;
        let va = self.off_to_va(at as u64)?;
        let pattern = if p == 8 {
            va.to_le_bytes().to_vec()
        } else {
            u32::try_from(va).ok()?.to_le_bytes().to_vec()
        };
        let mut search = 0;
        while let Some(found) = b[search..].windows(p).position(|w| w == pattern) {
            let pos = search + found;
            search = pos + 1;
            if pos % p != 0 {
                continue;
            }
            let named = |o: usize| self.read_ptr(o).and_then(|v| self.ident(v));
            let mut lo = pos;
            while lo >= p && named(lo - p).is_some() {
                lo -= p;
            }
            let mut hi = pos + p;
            while named(hi).is_some() {
                hi += p;
            }
            let slots: Vec<&str> = (lo..hi)
                .step_by(p)
                .map(|o| named(o).unwrap_or_default())
                .collect();
            let filled: Vec<usize> = (0..slots.len()).filter(|&k| !slots[k].is_empty()).collect();
            if filled.len() != count || count < 2 {
                continue;
            }
            if let Some(names) = index_array(b, &slots, &filled, count) {
                return Some(names);
            }
        }
        None
    }
}

/// Finds the u32 array (one entry per slot) whose filled slots hold
/// distinct indices `< count`, and orders the names by it.
fn index_array(b: &[u8], slots: &[&str], filled: &[usize], count: usize) -> Option<Vec<String>> {
    let n = slots.len();
    let at = |start: usize, k: usize| le32(b, start + k * 4).map_or(usize::MAX, |v| v as usize);
    let mut start = 0;
    while start + n * 4 <= b.len() {
        if at(start, filled[0]) < count && at(start, filled[1]) < count {
            let mut seen = vec![false; count];
            let good = filled.iter().all(|&k| {
                let v = at(start, k);
                v < count && !std::mem::replace(&mut seen[v], true)
            });
            if good {
                let mut names = vec![String::new(); count];
                for &k in filled {
                    names[at(start, k)] = slots[k].to_string();
                }
                return Some(names);
            }
        }
        start += 4;
    }
    None
}

/// ELF loadable segments (`PT_LOAD`), 32 or 64 bit, little endian.
fn elf_segments(b: &[u8]) -> Option<(Vec<Segment>, usize)> {
    if *b.get(5)? != 1 {
        return None; // big endian
    }
    let mut out = Vec::new();
    match *b.get(4)? {
        2 => {
            let phoff = le64(b, 0x20)? as usize;
            let (phentsize, phnum) = (le16(b, 0x36)? as usize, le16(b, 0x38)? as usize);
            for i in 0..phnum {
                let o = phoff + i * phentsize;
                if le32(b, o)? == 1 {
                    out.push(Segment {
                        off: le64(b, o + 0x08)?,
                        va: le64(b, o + 0x10)?,
                        size: le64(b, o + 0x20)?,
                    });
                }
            }
            Some((out, 8))
        }
        1 => {
            let phoff = le32(b, 0x1c)? as usize;
            let (phentsize, phnum) = (le16(b, 0x2a)? as usize, le16(b, 0x2c)? as usize);
            for i in 0..phnum {
                let o = phoff + i * phentsize;
                if le32(b, o)? == 1 {
                    out.push(Segment {
                        off: le32(b, o + 0x04)?.into(),
                        va: le32(b, o + 0x08)?.into(),
                        size: le32(b, o + 0x10)?.into(),
                    });
                }
            }
            Some((out, 4))
        }
        _ => None,
    }
}

/// PE sections at the preferred image base (PE32 or PE32+).
fn pe_segments(b: &[u8]) -> Option<(Vec<Segment>, usize)> {
    let pe = le32(b, 0x3c)? as usize;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let nsec = le16(b, pe + 6)? as usize;
    let optsz = le16(b, pe + 20)? as usize;
    let opt = pe + 24;
    let (base, ptr) = match le16(b, opt)? {
        0x20b => (le64(b, opt + 24)?, 8),
        0x10b => (le32(b, opt + 28)?.into(), 4),
        _ => return None,
    };
    let mut out = Vec::with_capacity(nsec);
    for i in 0..nsec {
        let s = opt + optsz + 40 * i;
        let vsize = le32(b, s + 8)? as u64;
        let rva = le32(b, s + 12)? as u64;
        let raw = le32(b, s + 16)? as u64;
        let rawptr = le32(b, s + 20)? as u64;
        let size = if vsize == 0 { raw } else { vsize.min(raw) };
        if size > 0 {
            out.push(Segment {
                va: base + rva,
                off: rawptr,
                size,
            });
        }
    }
    Some((out, ptr))
}

/// Mach-O segments (`LC_SEGMENT_64` / `LC_SEGMENT`) with file content.
fn macho_segments(b: &[u8]) -> Option<(Vec<Segment>, usize)> {
    let is64 = b.first()? == &0xcf;
    let ncmds = le32(b, 16)? as usize;
    let mut o = if is64 { 32 } else { 28 };
    let mut out = Vec::new();
    for _ in 0..ncmds {
        let (cmd, size) = (le32(b, o)?, le32(b, o + 4)? as usize);
        let seg = match cmd {
            0x19 => Some(Segment {
                va: le64(b, o + 24)?,
                off: le64(b, o + 40)?,
                size: le64(b, o + 48)?,
            }),
            0x1 => Some(Segment {
                va: le32(b, o + 24)?.into(),
                off: le32(b, o + 32)?.into(),
                size: le32(b, o + 36)?.into(),
            }),
            _ => None,
        };
        if let Some(s) = seg.filter(|s| s.size > 0) {
            out.push(s);
        }
        if size < 8 {
            return None;
        }
        o += size;
    }
    Some((out, if is64 { 8 } else { 4 }))
}

/// The architectures of a universal Mach-O.
fn fat_slices(b: &[u8]) -> Option<Vec<&[u8]>> {
    let wide = be32(b, 0)? == 0xcafe_babf;
    let n = be32(b, 4)? as usize;
    if n == 0 || n > 16 {
        return None; // also rules out Java class files
    }
    let mut out = Vec::new();
    for i in 0..n {
        let (off, size) = if wide {
            let e = 8 + 32 * i;
            (be64(b, e + 8)?, be64(b, e + 16)?)
        } else {
            let e = 8 + 20 * i;
            (be32(b, e + 8)?.into(), be32(b, e + 12)?.into())
        };
        let (off, size) = (usize::try_from(off).ok()?, usize::try_from(size).ok()?);
        out.push(b.get(off..off.checked_add(size)?)?);
    }
    Some(out)
}

/// The sound effect names in asset order (`count` = number of sound
/// assets in `Assets.dat`), from the bytes of any of the game's
/// executables; `None` when the table is not found.
pub fn sound_names(b: &[u8], count: usize) -> Option<Vec<String>> {
    if ExeFormat::detect(b)? == ExeFormat::MachOFat {
        return fat_slices(b)?
            .into_iter()
            .find_map(|s| sound_names(s, count));
    }
    Image::parse(b)?.sound_names(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lays out a fake image: names, then a pointer table with an empty
    /// slot, then the index array.
    fn table(ptr: usize, base: u64) -> (Vec<u8>, u64) {
        let mut data = vec![0u8; 16];
        let names = ["click", "winnery_fast", "pop"];
        let mut at = Vec::new();
        for n in names {
            at.push(base + data.len() as u64);
            data.extend_from_slice(n.as_bytes());
            data.push(0);
        }
        while !data.len().is_multiple_of(8) {
            data.push(0);
        }
        let empty = base + 15; // points at a NUL: an empty slot
        let slots = [at[2], empty, at[0], at[1]];
        for s in slots {
            data.extend_from_slice(&s.to_le_bytes()[..ptr]);
        }
        // indices: pop=2, -, click=0, winnery_fast=1
        for v in [2u32, 7, 0, 1] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        (data, base)
    }

    fn check(file: &[u8]) {
        assert_eq!(
            sound_names(file, 3).unwrap(),
            vec!["click", "winnery_fast", "pop"]
        );
    }

    #[test]
    fn elf64() {
        let (body, _) = table(8, 0x400000 + 0x80);
        let mut f = vec![0u8; 0x80];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 2;
        f[5] = 1;
        f[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
        f[0x36..0x38].copy_from_slice(&0x38u16.to_le_bytes());
        f[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
        let ph = 0x40;
        f[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        f[ph + 0x08..ph + 0x10].copy_from_slice(&0x80u64.to_le_bytes());
        f[ph + 0x10..ph + 0x18].copy_from_slice(&(0x400000u64 + 0x80).to_le_bytes());
        f[ph + 0x20..ph + 0x28].copy_from_slice(&(body.len() as u64).to_le_bytes());
        f.extend_from_slice(&body);
        assert_eq!(ExeFormat::detect(&f), Some(ExeFormat::Elf));
        check(&f);
    }

    fn pe(ptr: usize) -> Vec<u8> {
        let base = 0x1_4000_0000u64;
        let (rva, raw) = (0x1000u32, 0x200u32);
        let (body, _) = table(ptr, base + rva as u64);
        let mut f = vec![0u8; raw as usize];
        f[..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f[0x40..0x44].copy_from_slice(b"PE\0\0");
        f[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
        f[0x54..0x56].copy_from_slice(&0xf0u16.to_le_bytes());
        let opt = 0x58;
        if ptr == 8 {
            f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
            f[opt + 24..opt + 32].copy_from_slice(&base.to_le_bytes());
        } else {
            f[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
            f[opt + 28..opt + 32].copy_from_slice(&(base as u32).to_le_bytes());
        }
        let s = opt + 0xf0;
        f[s..s + 5].copy_from_slice(b".data");
        f[s + 8..s + 12].copy_from_slice(&(body.len() as u32).to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&rva.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&(body.len() as u32).to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&raw.to_le_bytes());
        f.extend_from_slice(&body);
        f
    }

    #[test]
    fn pe32_plus() {
        let f = pe(8);
        assert_eq!(ExeFormat::detect(&f), Some(ExeFormat::Pe));
        check(&f);
    }

    #[test]
    fn pe32() {
        // 32-bit image base 0x40000000 after truncation; still consistent.
        let f = pe(4);
        check(&f);
    }

    fn macho() -> Vec<u8> {
        let va = 0x1_0000_0000u64 + 0x100;
        let (body, _) = table(8, va);
        let mut f = vec![0u8; 0x100];
        f[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        f[16..20].copy_from_slice(&1u32.to_le_bytes());
        let c = 32;
        f[c..c + 4].copy_from_slice(&0x19u32.to_le_bytes());
        f[c + 4..c + 8].copy_from_slice(&72u32.to_le_bytes());
        f[c + 24..c + 32].copy_from_slice(&va.to_le_bytes());
        f[c + 32..c + 40].copy_from_slice(&(body.len() as u64).to_le_bytes());
        f[c + 40..c + 48].copy_from_slice(&0x100u64.to_le_bytes());
        f[c + 48..c + 56].copy_from_slice(&(body.len() as u64).to_le_bytes());
        f.extend_from_slice(&body);
        f
    }

    #[test]
    fn macho_thin_and_universal() {
        let thin = macho();
        assert_eq!(ExeFormat::detect(&thin), Some(ExeFormat::MachO));
        check(&thin);
        let mut fat = vec![0u8; 0x1000];
        fat[..4].copy_from_slice(&0xcafe_babeu32.to_be_bytes());
        fat[4..8].copy_from_slice(&1u32.to_be_bytes());
        fat[16..20].copy_from_slice(&0x1000u32.to_be_bytes());
        fat[20..24].copy_from_slice(&(thin.len() as u32).to_be_bytes());
        fat.extend_from_slice(&thin);
        assert_eq!(ExeFormat::detect(&fat), Some(ExeFormat::MachOFat));
        check(&fat);
    }

    #[test]
    fn rejects_other_files() {
        assert_eq!(sound_names(b"", 3), None);
        assert_eq!(sound_names(b"OggS\0\0\0\0", 3), None);
        assert_eq!(sound_names(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 50], 3), None);
        let mut f = macho();
        let n = f.len();
        f[n - 16..].fill(0); // index array gone
        assert_eq!(sound_names(&f, 3), None);
    }
}
