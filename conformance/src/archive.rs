// SPDX-License-Identifier: Apache-2.0
//! Opens compressed expected and produced outputs (`.gz`, `.zip`, `.tar.gz`,
//! `.tar.xz`) for comparison.
//!
//! This deliberately uses the archive crates' own readers (`tar::Archive`,
//! `zip::ZipArchive::by_index`), not RMLrs's streaming source readers, so
//! that a bug in RMLrs's writer and reader cannot cancel out.

use std::io::Read;
use std::path::Path;

/// The bytes of `path` and the name to read them by: the member name for
/// archives, the file name otherwise.
pub fn unpack(path: &Path) -> Result<(Vec<u8>, String), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".tar.gz") {
        let gz = flate2::read::MultiGzDecoder::new(bytes.as_slice());
        one_tar_member(gz)
    } else if name.ends_with(".tar.xz") {
        let xz = lzma_rust2::XzReader::new(bytes.as_slice(), true);
        one_tar_member(xz)
    } else if let Some(stem) = name.strip_suffix(".gz") {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(bytes.as_slice())
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        Ok((out, stem.to_owned()))
    } else if name.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let files: Vec<usize> = (0..z.len())
            .filter(|&i| z.by_index(i).is_ok_and(|f| f.is_file()))
            .collect();
        let [i] = files[..] else {
            return Err(format!("zip holds {} files, expected one", files.len()));
        };
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        f.read_to_end(&mut out).map_err(|e| e.to_string())?;
        let name = f.name().map_err(|e| e.to_string())?.into_owned();
        Ok((out, name))
    } else {
        Ok((bytes, name))
    }
}

fn one_tar_member(r: impl Read) -> Result<(Vec<u8>, String), String> {
    let mut archive = tar::Archive::new(r);
    let mut found = None;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        if found.is_some() {
            return Err("tar holds more than one file".into());
        }
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let mut out = Vec::new();
        entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
        found = Some((out, name));
    }
    found.ok_or_else(|| "tar holds no file".into())
}
