//! Upload and report (`docs/spec/levelcodes.md` §6, §7). Only built with
//! the `upload` feature, which is off by default: uploading to Hempuli's
//! server needs the owner's explicit approval. Tested against the local mock
//! only.

use std::io::Write;

use super::LcError;
use super::net::Server;

/// The multipart boundary the original uses (§6.2).
pub const BOUNDARY: &str = "WrbKitFormBoundaryXtRo8bh7ZpnTrTwd";

/// A ZIP archive (deflate) of `entries` in order.
pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).expect("in memory");
        let comp = enc.finish().expect("in memory");
        let offset = out.len() as u32;
        let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
        let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        // Local file header.
        le32(&mut out, 0x0403_4b50);
        le16(&mut out, 20); // version needed
        le16(&mut out, 0); // flags
        le16(&mut out, 8); // deflate
        le16(&mut out, 0); // time
        le16(&mut out, 0x21); // date 1980-01-01
        le32(&mut out, crc);
        le32(&mut out, comp.len() as u32);
        le32(&mut out, data.len() as u32);
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&comp);
        // Central directory entry.
        le32(&mut central, 0x0201_4b50);
        le16(&mut central, 20);
        le16(&mut central, 20);
        le16(&mut central, 0);
        le16(&mut central, 8);
        le16(&mut central, 0);
        le16(&mut central, 0x21);
        le32(&mut central, crc);
        le32(&mut central, comp.len() as u32);
        le32(&mut central, data.len() as u32);
        le16(&mut central, name.len() as u16);
        le16(&mut central, 0); // extra
        le16(&mut central, 0); // comment
        le16(&mut central, 0); // disk
        le16(&mut central, 0); // internal attrs
        le32(&mut central, 0); // external attrs
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    let n = entries.len() as u16;
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// The upload body (§6.2): `difficulty`, `author`, `name`, then `file`
/// (a zip of `level.ld` and `level.l`).
pub fn multipart(difficulty: &str, author: &str, name: &str, ld: &[u8], l: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for (k, v) in [
        ("difficulty", difficulty),
        ("author", author),
        ("name", name),
    ] {
        b.extend_from_slice(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    b.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file\"\r\n\r\n"
        )
        .as_bytes(),
    );
    b.extend_from_slice(&zip(&[("level.ld", ld), ("level.l", l)]));
    b.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    b
}

impl Server {
    /// `POST upload_level` (§6.2); the body of a success is the new code.
    pub fn upload(
        &self,
        difficulty: &str,
        author: &str,
        name: &str,
        ld: &[u8],
        l: &[u8],
    ) -> Result<String, LcError> {
        if self.offline {
            return Err(LcError::Network);
        }
        let url = self.api("upload_level", "");
        log::warn!("level server: UPLOAD {url}");
        let body = multipart(difficulty, author, name, ld, l);
        let r = baba_http::post(
            &url,
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            &body,
            self.timeout,
        )
        .map_err(|e| match e {
            baba_http::Error::Timeout => LcError::Timeout,
            baba_http::Error::Network(_) => LcError::Network,
        })?;
        if r.status != 200 {
            return Err(super::status_error(r.status, &r.body));
        }
        super::normalize_code(r.text().trim()).ok_or(LcError::Invalid)
    }

    /// `GET report_level` (§7.1).
    pub fn report(&self, code: &str) -> Result<(), LcError> {
        if self.offline {
            return Err(LcError::Network);
        }
        let code = super::normalize_code(code).ok_or(LcError::Invalid)?;
        let r = self.get(&self.api("report_level", &format!("&level={code}")))?;
        if r.status != 200 {
            return Err(super::status_error(r.status, &r.body));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock;
    use super::*;

    #[test]
    fn zip_is_well_formed() {
        let z = zip(&[
            ("level.ld", b"[general]\nname=A\n"),
            ("level.l", b"ACHTUNG!"),
        ]);
        assert_eq!(&z[..4], b"PK\x03\x04");
        assert_eq!(&z[z.len() - 22..z.len() - 18], b"PK\x05\x06");
        // Entry order: .ld first.
        assert_eq!(&z[30..38], b"level.ld");
        // Inflate the first entry back.
        let comp_len = u32::from_le_bytes(z[18..22].try_into().unwrap()) as usize;
        let mut d = flate2::read::DeflateDecoder::new(&z[38..38 + comp_len]);
        let mut s = String::new();
        std::io::Read::read_to_string(&mut d, &mut s).unwrap();
        assert_eq!(s, "[general]\nname=A\n");
    }

    #[test]
    fn upload_and_report_against_the_mock() {
        let m = mock::start();
        m.routes.lock().unwrap().insert(
            "/prod/upload_level?ver=2".into(),
            (200, b"QWER-7890".to_vec()),
        );
        m.routes.lock().unwrap().insert(
            "/prod/report_level?ver=2&level=QWER-7890".into(),
            (200, Vec::new()),
        );
        let s = Server::at(&m.base);
        let code = s.upload("sub", "me", "Lvl", b"LD", b"L").unwrap();
        assert_eq!(code, "QWER-7890");
        s.report("qwer7890").unwrap();
        let seen = m.seen.lock().unwrap();
        assert!(seen[0].0.starts_with("POST /prod/upload_level?ver=2 "));
        let body = String::from_utf8_lossy(&seen[0].1);
        assert!(body.starts_with(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"difficulty\"\r\n\r\nsub\r\n"
        )));
        assert!(body.contains("name=\"file\"; filename=\"file\""));
        assert!(body.ends_with(&format!("--{BOUNDARY}--\r\n")));
        assert!(
            seen[1]
                .0
                .starts_with("GET /prod/report_level?ver=2&level=QWER-7890 ")
        );
        drop(seen);
        m.routes
            .lock()
            .unwrap()
            .insert("/prod/upload_level?ver=2".into(), (512, Vec::new()));
        assert_eq!(
            s.upload("", "me", "Lvl", b"", b"").unwrap_err(),
            LcError::SpriteError
        );
    }
}
