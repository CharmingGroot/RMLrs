// SPDX-License-Identifier: Apache-2.0
//! `rml:compression` of file sources (RML-IO): GZip, Zip, Tar+GZip and
//! Tar+Xz. Every format is read as a stream, so a compressed CSV file is
//! still processed row by row.
//!
//! An archive (Zip or Tar) must hold exactly one file: RML-IO has no way to
//! say which member of an archive is the source.

use std::fs::File;
use std::io::BufRead;
#[cfg(feature = "compression")]
use std::io::{self, BufReader, Read, Seek, SeekFrom};

use rmlrs_model::{Compression, Error, Result};

/// Wraps an opened source file so that reading yields the decompressed data.
pub fn open(file: File, compression: Compression) -> Result<Box<dyn BufRead>> {
    imp::open(file, compression)
}

#[cfg(not(feature = "compression"))]
mod imp {
    use super::*;

    pub fn open(file: File, compression: Compression) -> Result<Box<dyn BufRead>> {
        match compression {
            Compression::None => Ok(Box::new(std::io::BufReader::with_capacity(1 << 16, file))),
            other => Err(Error::Unsupported(format!(
                "rml:compression {other:?} (build with the \"compression\" feature)"
            ))),
        }
    }
}

#[cfg(feature = "compression")]
mod imp {
    use super::*;
    use flate2::read::{DeflateDecoder, MultiGzDecoder};

    const BUF: usize = 1 << 16;

    pub fn open(file: File, compression: Compression) -> Result<Box<dyn BufRead>> {
        let file = BufReader::with_capacity(BUF, file);
        Ok(match compression {
            Compression::None => Box::new(file),
            Compression::Gzip => Box::new(BufReader::with_capacity(BUF, MultiGzDecoder::new(file))),
            Compression::Zip => zip_member(file.into_inner())?,
            Compression::TarGz => Box::new(BufReader::with_capacity(
                BUF,
                TarMember::new(MultiGzDecoder::new(file))?,
            )),
            Compression::TarXz => Box::new(BufReader::with_capacity(
                BUF,
                TarMember::new(lzma_rust2::XzReader::new(file, true))?,
            )),
        })
    }

    fn source_error(e: impl std::fmt::Display) -> Error {
        Error::Source(e.to_string())
    }

    /// The only file of a Zip archive, read straight from its offset in the
    /// file so that the archive does not have to stay borrowed. Stored and
    /// Deflate members are supported; size and CRC-32 are checked at the end.
    fn zip_member(file: File) -> Result<Box<dyn BufRead>> {
        use zip::CompressionMethod;

        let mut archive = zip::ZipArchive::new(file).map_err(source_error)?;
        let mut member = None;
        let mut files = 0;
        for i in 0..archive.len() {
            let f = archive.by_index_raw(i).map_err(source_error)?;
            if f.is_file() {
                files += 1;
                member = Some((
                    f.compression(),
                    f.encrypted(),
                    f.data_start(),
                    f.compressed_size(),
                    f.size(),
                    f.crc32(),
                ));
            }
        }
        let Some((method, encrypted, start, compressed, size, crc)) = member.filter(|_| files == 1)
        else {
            return Err(Error::Source(format!(
                "zip archive holds {files} files; a compressed source must hold exactly one"
            )));
        };
        if encrypted {
            return Err(Error::Unsupported("encrypted zip archive".into()));
        }
        let start =
            start.ok_or_else(|| Error::Source("zip member without a data offset".into()))?;
        let mut file = archive.into_inner();
        file.seek(SeekFrom::Start(start)).map_err(source_error)?;
        let raw = BufReader::with_capacity(BUF, file).take(compressed);
        let data: Box<dyn Read> = match method {
            CompressionMethod::Stored => Box::new(raw),
            CompressionMethod::Deflated => Box::new(DeflateDecoder::new(raw)),
            other => {
                return Err(Error::Unsupported(format!(
                    "zip member compressed with {other}"
                )));
            }
        };
        Ok(Box::new(BufReader::with_capacity(
            BUF,
            Checked {
                inner: data,
                crc: flate2::Crc::new(),
                expected_crc: crc,
                expected_size: size,
            },
        )))
    }

    /// Checks size and CRC-32 of a Zip member once it has been read to the end.
    struct Checked<R> {
        inner: R,
        crc: flate2::Crc,
        expected_crc: u32,
        expected_size: u64,
    }

    impl<R: Read> Read for Checked<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            if n == 0 && !buf.is_empty() {
                if u64::from(self.crc.amount()) != self.expected_size & 0xFFFF_FFFF
                    || self.crc.sum() != self.expected_crc
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "zip member is corrupt (size or CRC-32 mismatch)",
                    ));
                }
            } else {
                self.crc.update(&buf[..n]);
            }
            Ok(n)
        }
    }

    /// The only regular file of a Tar stream. Reading past its end checks
    /// that no other file follows.
    struct TarMember<R> {
        inner: R,
        remaining: u64,
        padding: u64,
        checked_rest: bool,
    }

    const BLOCK: usize = 512;

    impl<R: Read> TarMember<R> {
        fn new(mut inner: R) -> Result<Self> {
            let size = next_file(&mut inner)
                .map_err(source_error)?
                .ok_or_else(|| Error::Source("tar archive holds no file".into()))?;
            Ok(TarMember {
                inner,
                remaining: size,
                padding: padding(size),
                checked_rest: false,
            })
        }
    }

    impl<R: Read> Read for TarMember<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                if !self.checked_rest {
                    self.checked_rest = true;
                    skip(&mut self.inner, self.padding)?;
                    if next_file(&mut self.inner)?.is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "tar archive holds more than one file; a compressed source must hold exactly one",
                        ));
                    }
                }
                return Ok(0);
            }
            let max = buf
                .len()
                .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
            let n = self.inner.read(&mut buf[..max])?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.remaining -= n as u64;
            Ok(n)
        }
    }

    fn padding(size: u64) -> u64 {
        (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64
    }

    fn skip(r: &mut impl Read, n: u64) -> io::Result<()> {
        let copied = io::copy(&mut r.by_ref().take(n), &mut io::sink())?;
        if copied == n {
            Ok(())
        } else {
            Err(io::ErrorKind::UnexpectedEof.into())
        }
    }

    /// Reads headers up to the next regular file and returns its size, or
    /// `None` at the end of the archive. Directories and PAX / GNU metadata
    /// entries are skipped.
    fn next_file(r: &mut impl Read) -> io::Result<Option<u64>> {
        let mut block = [0u8; BLOCK];
        loop {
            if !read_block(r, &mut block)? || block.iter().all(|&b| b == 0) {
                return Ok(None);
            }
            let header = tar::Header::from_byte_slice(&block);
            if header.cksum()? != checksum(&block) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "tar header checksum mismatch",
                ));
            }
            let size = header.entry_size()?;
            let kind = header.entry_type();
            if kind.is_file() || kind == tar::EntryType::Continuous {
                return Ok(Some(size));
            }
            if kind.is_dir()
                || kind.is_pax_global_extensions()
                || kind.is_pax_local_extensions()
                || kind.is_gnu_longname()
                || kind.is_gnu_longlink()
            {
                skip(r, size + padding(size))?;
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tar entry of type {kind:?} is not a regular file"),
            ));
        }
    }

    /// Fills a whole block; `false` at a clean end of stream.
    fn read_block(r: &mut impl Read, block: &mut [u8; BLOCK]) -> io::Result<bool> {
        let mut filled = 0;
        while filled < BLOCK {
            match r.read(&mut block[filled..])? {
                0 if filled == 0 => return Ok(false),
                0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                n => filled += n,
            }
        }
        Ok(true)
    }

    /// The header checksum: the sum of all bytes, with the checksum field
    /// itself read as spaces.
    fn checksum(block: &[u8; BLOCK]) -> u32 {
        block
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if (148..156).contains(&i) {
                    u32::from(b' ')
                } else {
                    u32::from(b)
                }
            })
            .sum()
    }
}

#[cfg(all(test, feature = "compression"))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn temp(name: &str, bytes: &[u8]) -> File {
        let dir = std::env::temp_dir().join(format!("rmlrs-source-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        File::open(path).unwrap()
    }

    fn read_all(file: File, c: Compression) -> Result<String> {
        let mut s = String::new();
        open(file, c)?
            .read_to_string(&mut s)
            .map_err(|e| Error::Source(e.to_string()))?;
        Ok(s)
    }

    fn tar_of(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in members {
            let mut h = tar::Header::new_ustar();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn gzip_and_tar_round_trip() {
        let data = b"id,name\n1,Venus\n";
        let gz = temp("a.csv.gz", &gzip(data));
        assert_eq!(
            read_all(gz, Compression::Gzip).unwrap(),
            "id,name\n1,Venus\n"
        );

        // A long member crosses several blocks and needs padding.
        let long = "x".repeat(1500);
        let tgz = temp("a.tar.gz", &gzip(&tar_of(&[("a.csv", long.as_bytes())])));
        assert_eq!(read_all(tgz, Compression::TarGz).unwrap(), long);

        let mut xz =
            lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(6)).unwrap();
        xz.write_all(&tar_of(&[("dir/a.json", b"[1]")])).unwrap();
        let txz = temp("a.tar.xz", &xz.finish().unwrap());
        assert_eq!(read_all(txz, Compression::TarXz).unwrap(), "[1]");
    }

    #[test]
    fn archives_must_hold_one_file() {
        let two = temp(
            "two.tar.gz",
            &gzip(&tar_of(&[("a.csv", b"a\n"), ("b.csv", b"b\n")])),
        );
        assert!(matches!(
            read_all(two, Compression::TarGz),
            Err(Error::Source(m)) if m.contains("more than one file")
        ));

        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("a.json", opts).unwrap();
        z.write_all(b"[]").unwrap();
        z.start_file("b.json", opts).unwrap();
        let two_zip = temp("two.zip", &z.finish().unwrap().into_inner());
        assert!(matches!(
            read_all(two_zip, Compression::Zip),
            Err(Error::Source(m)) if m.contains("2 files")
        ));
    }

    #[test]
    fn zip_member_is_checked() {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        z.add_directory("d/", opts).unwrap();
        z.start_file("d/a.json", opts).unwrap();
        z.write_all(b"{\"a\": 1}").unwrap();
        let mut bytes = z.finish().unwrap().into_inner();
        let ok = temp("ok.zip", &bytes);
        assert_eq!(read_all(ok, Compression::Zip).unwrap(), "{\"a\": 1}");

        // Flip one byte of the stored data: the CRC check must catch it.
        let at = bytes.windows(3).position(|w| w == b"\"a\"").unwrap();
        bytes[at + 1] = b'b';
        let bad = temp("bad.zip", &bytes);
        assert!(read_all(bad, Compression::Zip).is_err());
    }
}
