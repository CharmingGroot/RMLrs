// SPDX-License-Identifier: Apache-2.0
//! `rml:compression` of logical targets (RML-IO): GZip, Zip, Tar+GZip and
//! Tar+Xz.
//!
//! Output is deterministic: archive members carry a fixed time (the Unix
//! epoch for Tar, 1980-01-01 for Zip) and mode 0644, and GZip headers have no
//! time or name. The member of an archive is named after the target file
//! without the compression extension (`dump.nq.tar.gz` holds `dump.nq`).
//!
//! GZip and Zip stream straight to the file. A Tar header needs the member
//! size first, so Tar output goes to a temporary file next to the target and
//! is packed when the target is finished.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rmlrs_model::{Compression, Error, Result};

/// The byte sink of one target file.
#[derive(Debug)]
pub(crate) struct Sink(imp::Inner);

impl Sink {
    pub(crate) fn create(path: &Path, compression: Compression) -> Result<Self> {
        imp::create(path, compression).map(Sink)
    }

    pub(crate) fn finish(self) -> io::Result<()> {
        imp::finish(self.0)
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        imp::write(&mut self.0, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        imp::flush(&mut self.0)
    }
}

fn create_file(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| target_error(path, e))?;
    }
    File::create(path).map_err(|e| target_error(path, e))
}

fn target_error(path: &Path, e: io::Error) -> Error {
    Error::Source(format!("target {}: {e}", path.display()))
}

/// The name of the single member of an archive written to `path`.
#[cfg_attr(not(feature = "compression"), allow(dead_code))]
pub(crate) fn member_name(path: &Path, compression: Compression) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let suffixes: &[&str] = match compression {
        Compression::None => &[],
        Compression::Gzip => &[".gz"],
        Compression::Zip => &[".zip"],
        Compression::TarGz => &[".tar.gz", ".tgz"],
        Compression::TarXz => &[".tar.xz", ".txz"],
    };
    for s in suffixes {
        if let Some(stem) = name.strip_suffix(s).filter(|s| !s.is_empty()) {
            return stem.to_owned();
        }
    }
    name
}

#[cfg_attr(not(feature = "compression"), allow(dead_code))]
fn part_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.rmlrs-part"))
}

#[cfg(not(feature = "compression"))]
mod imp {
    use super::*;

    pub(crate) type Inner = File;

    pub(crate) fn create(path: &Path, compression: Compression) -> Result<Inner> {
        match compression {
            Compression::None => create_file(path),
            other => Err(Error::Unsupported(format!(
                "rml:compression {other:?} (build with the \"compression\" feature)"
            ))),
        }
    }

    pub(crate) fn write(f: &mut Inner, buf: &[u8]) -> io::Result<usize> {
        f.write(buf)
    }

    pub(crate) fn flush(f: &mut Inner) -> io::Result<()> {
        f.flush()
    }

    pub(crate) fn finish(mut f: Inner) -> io::Result<()> {
        f.flush()
    }
}

#[cfg(feature = "compression")]
mod imp {
    use super::*;
    use flate2::write::GzEncoder;
    use std::io::{Seek, SeekFrom};

    pub(crate) enum Inner {
        Plain(File),
        Gzip(GzEncoder<File>),
        Zip(Box<zip::ZipWriter<File>>),
        Tar {
            part: File,
            part_path: PathBuf,
            path: PathBuf,
            member: String,
            xz: bool,
        },
    }

    impl std::fmt::Debug for Inner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(match self {
                Inner::Plain(_) => "Plain",
                Inner::Gzip(_) => "Gzip",
                Inner::Zip(_) => "Zip",
                Inner::Tar { xz: false, .. } => "TarGz",
                Inner::Tar { xz: true, .. } => "TarXz",
            })
        }
    }

    fn gzip(f: File) -> GzEncoder<File> {
        GzEncoder::new(f, flate2::Compression::default())
    }

    pub(crate) fn create(path: &Path, compression: Compression) -> Result<Inner> {
        let file = create_file(path)?;
        Ok(match compression {
            Compression::None => Inner::Plain(file),
            Compression::Gzip => Inner::Gzip(gzip(file)),
            Compression::Zip => {
                let mut z = zip::ZipWriter::new(file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .last_modified_time(zip::DateTime::default())
                    .unix_permissions(0o644)
                    .large_file(true);
                z.start_file(member_name(path, compression), options)
                    .map_err(|e| Error::Source(format!("target {}: {e}", path.display())))?;
                Inner::Zip(Box::new(z))
            }
            Compression::TarGz | Compression::TarXz => {
                let part_path = part_path(path);
                // Written now, read back when packing.
                let part = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&part_path)
                    .map_err(|e| target_error(&part_path, e))?;
                Inner::Tar {
                    part,
                    part_path,
                    path: path.to_owned(),
                    member: member_name(path, compression),
                    xz: compression == Compression::TarXz,
                }
            }
        })
    }

    pub(crate) fn write(s: &mut Inner, buf: &[u8]) -> io::Result<usize> {
        match s {
            Inner::Plain(f) | Inner::Tar { part: f, .. } => f.write(buf),
            Inner::Gzip(g) => g.write(buf),
            Inner::Zip(z) => z.write(buf),
        }
    }

    pub(crate) fn flush(s: &mut Inner) -> io::Result<()> {
        match s {
            Inner::Plain(f) | Inner::Tar { part: f, .. } => f.flush(),
            Inner::Gzip(g) => g.flush(),
            Inner::Zip(z) => z.flush(),
        }
    }

    pub(crate) fn finish(s: Inner) -> io::Result<()> {
        match s {
            Inner::Plain(mut f) => f.flush(),
            Inner::Gzip(g) => g.finish()?.flush(),
            Inner::Zip(z) => z.finish().map_err(io::Error::other)?.flush(),
            Inner::Tar {
                mut part,
                part_path,
                path,
                member,
                xz,
            } => {
                let r = pack(&mut part, &path, &member, xz);
                drop(part);
                std::fs::remove_file(&part_path).ok();
                r
            }
        }
    }

    fn pack(part: &mut File, path: &Path, member: &str, xz: bool) -> io::Result<()> {
        part.flush()?;
        let size = part.seek(SeekFrom::End(0))?;
        part.seek(SeekFrom::Start(0))?;
        let mut header = tar::Header::new_ustar();
        header.set_size(size);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        let out = File::create(path)?;
        if xz {
            let enc = lzma_rust2::XzWriter::new(out, lzma_rust2::XzOptions::with_preset(6))?;
            let mut b = tar::Builder::new(enc);
            b.append_data(&mut header, member, part)?;
            b.into_inner()?.finish()?.flush()
        } else {
            let mut b = tar::Builder::new(gzip(out));
            b.append_data(&mut header, member, part)?;
            b.into_inner()?.finish()?.flush()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_names() {
        let n = |p: &str, c| member_name(Path::new(p), c);
        assert_eq!(n("out/dump1.nq.tar.xz", Compression::TarXz), "dump1.nq");
        assert_eq!(n("dump1.nq.tgz", Compression::TarGz), "dump1.nq");
        assert_eq!(n("dump1.nq.zip", Compression::Zip), "dump1.nq");
        assert_eq!(n("dump1.nq.gz", Compression::Gzip), "dump1.nq");
        // No matching extension: the member keeps the file name.
        assert_eq!(n("dump1.nq", Compression::Zip), "dump1.nq");
        assert_eq!(n(".zip", Compression::Zip), ".zip");
    }
}
