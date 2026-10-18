//! Little-endian byte reader with offset-carrying errors.

use crate::error::{Error, Result};

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    format: &'static str,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], format: &'static str) -> Self {
        Reader {
            data,
            pos: 0,
            format,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn err(&self, message: impl Into<String>) -> Error {
        Error::malformed(self.format, Some(self.pos), message)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(self.err(format!("need {n} bytes, only {} left", self.remaining())));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn peek(&self, n: usize) -> Option<&'a [u8]> {
        self.data.get(self.pos..self.pos + n)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.bytes(N)?);
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
}

/// zlib (RFC 1950) decompression.
pub(crate) fn zlib_decompress(data: &[u8], format: &'static str) -> Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib(data)
        .map_err(|e| Error::decode(format, format!("zlib: {e:?}")))
}

/// zlib (RFC 1950) compression at the default level.
pub(crate) fn zlib_compress(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}
