//! Compressed SQL dumps (`.sql.gz`, `.zip`): format detection and the streaming (de)compression
//! used by [`crate::supervisor`]'s piped commands. Data moves in fixed-size chunks; a dump is
//! never held in memory.
//!
//! Detection trusts the content (magic bytes) over the name; a name that promises a format the
//! content does not have, a tar archive or another compressor (bzip2, xz, zstd, 7-Zip, RAR) is
//! rejected with a message that says what to do.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use lampo_domain::ports::StreamCompression;
use lampo_domain::{PortError, PortResult};
use zip::ZipArchive;

pub(crate) const CHUNK: usize = 64 * 1024;

const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];
const ZIP_MAGICS: [&[u8]; 2] = [b"PK\x03\x04", b"PK\x05\x06"];
const UNSUPPORTED_MAGICS: [(&[u8], &str); 5] =
    [(b"BZh", "bzip2"), (&[0xfd, b'7', b'z', b'X', b'Z', 0], "xz"), (&[0x28, 0xb5, 0x2f, 0xfd], "zstd"), (&[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c], "7-Zip"), (b"Rar!", "RAR")];

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn lower_name(path: &Path) -> String {
    display_name(path).to_ascii_lowercase()
}

fn open(path: &Path) -> PortResult<File> {
    File::open(path)
        .map_err(|e| if e.kind() == io::ErrorKind::NotFound { PortError::not_found("file", path.display().to_string()) } else { PortError::io(format!("opening {}", path.display()), e) })
}

/// The compression of a dump to import, from its first bytes (the extension only confirms).
/// A zip must hold exactly one `.sql` file. `NotFound` if the file is missing.
pub fn detect_input(path: &Path) -> PortResult<StreamCompression> {
    let mut head = Vec::with_capacity(8);
    open(path)?.take(8).read_to_end(&mut head).map_err(|e| PortError::io(format!("reading {}", path.display()), e))?;
    let name = lower_name(path);
    let invalid = |message: String| PortError::invalid_data(display_name(path), message);
    if let Some((_, kind)) = UNSUPPORTED_MAGICS.iter().find(|(magic, _)| head.starts_with(magic)) {
        return Err(invalid(format!("{kind}-compressed dumps are not supported: decompress it to a .sql file first (or recompress it as .sql.gz or .zip)")));
    }
    if name.ends_with(".tar") || name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        return Err(invalid("tar archives are not supported: extract the .sql file first (`tar -xf`), or use a .sql.gz or a .zip".into()));
    }
    if head.starts_with(GZIP_MAGIC) {
        return Ok(StreamCompression::Gzip);
    }
    if ZIP_MAGICS.iter().any(|m| head.starts_with(m)) {
        let mut archive = ZipArchive::new(BufReader::new(open(path)?)).map_err(|e| invalid(format!("not a readable zip archive: {e}")))?;
        sql_entry(&mut archive, path)?;
        return Ok(StreamCompression::Zip);
    }
    if name.ends_with(".gz") {
        return Err(invalid("the name ends in .gz but the content is not gzip-compressed".into()));
    }
    if name.ends_with(".zip") {
        return Err(invalid("the name ends in .zip but the content is not a zip archive".into()));
    }
    Ok(StreamCompression::None)
}

/// The compression of an export destination, from its name: `.gz` is gzip, anything else plain.
pub fn detect_output(path: &Path) -> PortResult<StreamCompression> {
    let name = lower_name(path);
    if name.ends_with(".zip") {
        return Err(PortError::invalid_data(display_name(path), "exporting to a .zip is not supported: use .sql.gz (or .sql)"));
    }
    Ok(if name.ends_with(".gz") { StreamCompression::Gzip } else { StreamCompression::None })
}

/// Index of the archive's single `.sql` file (folders and macOS `__MACOSX/` metadata ignored).
fn sql_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, path: &Path) -> PortResult<usize> {
    let candidates: Vec<(usize, String)> = (0..archive.len())
        .filter_map(|i| archive.name_for_index(i).map(|n| (i, n.to_string())))
        .filter(|(_, n)| !n.ends_with('/') && !n.starts_with("__MACOSX/") && n.to_ascii_lowercase().ends_with(".sql"))
        .collect();
    match candidates.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err(PortError::invalid_data(display_name(path), "the zip archive contains no .sql file")),
        many => {
            let names: Vec<&str> = many.iter().map(|(_, n)| n.as_str()).collect();
            Err(PortError::invalid_data(
                display_name(path),
                format!("the zip archive contains {} .sql files ({}): import them one at a time after extracting them", names.len(), names.join(", ")),
            ))
        }
    }
}

/// A reader that publishes its position in the underlying file (progress of a compressed import:
/// bytes of the *compressed* file consumed).
struct PositionReader<R> {
    inner: R,
    position: Arc<AtomicU64>,
}

impl<R: Read> Read for PositionReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.position.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl<R: Seek> Seek for PositionReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = self.inner.seek(to)?;
        self.position.store(at, Ordering::Relaxed);
        Ok(at)
    }
}

/// Streams the decompressed content of `path` into `sink` chunk by chunk (blocking: run it on a
/// blocking thread). `sink` returning `false` (the consumer is gone) stops quietly. `Err` is a
/// message for corrupt or unreadable data. `position` ends at the file size on success.
pub(crate) fn decode(path: &Path, compression: StreamCompression, position: &Arc<AtomicU64>, mut sink: impl FnMut(Vec<u8>) -> bool) -> Result<(), String> {
    let file = open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let counted = PositionReader { inner: file, position: position.clone() };
    let finished = match compression {
        StreamCompression::None => pump(counted, &mut sink),
        StreamCompression::Gzip => pump(MultiGzDecoder::new(counted), &mut sink),
        StreamCompression::Zip => {
            let mut archive = ZipArchive::new(BufReader::new(counted)).map_err(|e| format!("not a readable zip archive: {e}"))?;
            let index = sql_entry(&mut archive, path).map_err(|e| e.to_string())?;
            let entry = archive.by_index(index).map_err(|e| format!("cannot read the zip entry: {e}"))?;
            pump(entry, &mut sink)
        }
    };
    match finished {
        Ok(true) => {
            position.store(size, Ordering::Relaxed);
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => Err(format!("corrupt or truncated {} data: {e}", label(compression))),
    }
}

fn label(compression: StreamCompression) -> &'static str {
    match compression {
        StreamCompression::None => "dump",
        StreamCompression::Gzip => "gzip",
        StreamCompression::Zip => "zip",
    }
}

/// `Ok(true)` at end of input, `Ok(false)` when the sink stopped accepting.
fn pump(mut reader: impl Read, sink: &mut impl FnMut(Vec<u8>) -> bool) -> io::Result<bool> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if !sink(buf[..n].to_vec()) {
            return Ok(false);
        }
    }
}

/// Writes the chunks `next` yields (until `None`) gzip-compressed into `file` (blocking).
pub(crate) fn encode_gzip(file: File, mut next: impl FnMut() -> Option<Vec<u8>>) -> io::Result<()> {
    let mut encoder = GzEncoder::new(BufWriter::new(file), flate2::Compression::default());
    while let Some(chunk) = next() {
        encoder.write_all(&chunk)?;
    }
    encoder.finish()?.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SQL: &str = "CREATE TABLE t (id INT);\nINSERT INTO t VALUES (1),(2),(3);\n";

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    fn zip_file(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(name);
        let mut w = zip::ZipWriter::new(File::create(&path).unwrap());
        for (entry, content) in entries {
            if entry.ends_with('/') {
                w.add_directory(*entry, zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*entry, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(content).unwrap();
            }
        }
        w.finish().unwrap();
        path
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn decoded(path: &Path, compression: StreamCompression) -> (Result<Vec<u8>, String>, u64) {
        let position = Arc::new(AtomicU64::new(0));
        let mut out = Vec::new();
        let result = decode(path, compression, &position, |c| {
            out.extend_from_slice(&c);
            true
        });
        (result.map(|()| out), position.load(Ordering::Relaxed))
    }

    #[test]
    fn plain_sql_is_not_compressed_whatever_its_name() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_input(&write(dir.path(), "dump.sql", SQL.as_bytes())).unwrap(), StreamCompression::None);
        assert_eq!(detect_input(&write(dir.path(), "dump.txt", SQL.as_bytes())).unwrap(), StreamCompression::None);
        assert_eq!(detect_input(&write(dir.path(), "empty.sql", b"")).unwrap(), StreamCompression::None);
    }

    #[test]
    fn gzip_is_detected_by_content_and_streamed_back() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = gzip(SQL.as_bytes());
        for name in ["dump.sql.gz", "dump.gz", "DUMP.SQL.GZ", "dump.sql"] {
            let path = write(dir.path(), name, &bytes);
            assert_eq!(detect_input(&path).unwrap(), StreamCompression::Gzip, "{name}");
        }
        let path = dir.path().join("dump.sql.gz");
        let (out, position) = decoded(&path, StreamCompression::Gzip);
        assert_eq!(out.unwrap(), SQL.as_bytes());
        assert_eq!(position, bytes.len() as u64, "progress = compressed bytes read");
    }

    #[test]
    fn concatenated_gzip_members_are_all_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = gzip(b"SELECT 1;\n");
        bytes.extend(gzip(b"SELECT 2;\n"));
        let path = write(dir.path(), "multi.sql.gz", &bytes);
        assert_eq!(decoded(&path, StreamCompression::Gzip).0.unwrap(), b"SELECT 1;\nSELECT 2;\n");
    }

    #[test]
    fn truncated_gzip_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let big: String = (0..20_000).map(|i| format!("INSERT INTO t VALUES ({i});\n")).collect();
        let bytes = gzip(big.as_bytes());
        let path = write(dir.path(), "cut.sql.gz", &bytes[..bytes.len() / 2]);
        assert_eq!(detect_input(&path).unwrap(), StreamCompression::Gzip);
        let err = decoded(&path, StreamCompression::Gzip).0.unwrap_err();
        assert!(err.contains("corrupt or truncated gzip"), "{err}");
    }

    #[test]
    fn zip_with_one_sql_file_streams_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = zip_file(dir.path(), "dump.zip", &[("backup/", b""), ("backup/README.txt", b"hello"), ("__MACOSX/backup/._shop.sql", b"junk"), ("backup/shop.SQL", SQL.as_bytes())]);
        assert_eq!(detect_input(&path).unwrap(), StreamCompression::Zip);
        let (out, position) = decoded(&path, StreamCompression::Zip);
        assert_eq!(out.unwrap(), SQL.as_bytes());
        assert_eq!(position, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn zip_without_exactly_one_sql_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let none = zip_file(dir.path(), "none.zip", &[("readme.txt", b"x")]);
        let err = detect_input(&none).unwrap_err().to_string();
        assert!(err.contains("no .sql file"), "{err}");
        let many = zip_file(dir.path(), "many.zip", &[("a.sql", b"SELECT 1;"), ("b/b.sql", b"SELECT 2;")]);
        let err = detect_input(&many).unwrap_err().to_string();
        assert!(err.contains("2 .sql files (a.sql, b/b.sql)"), "{err}");
        assert!(matches!(detect_input(&many), Err(PortError::InvalidData { .. })));
    }

    #[test]
    fn broken_or_mislabelled_archives_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = detect_input(&write(dir.path(), "bad.zip", b"PK\x03\x04 definitely not a zip")).unwrap_err().to_string();
        assert!(err.contains("not a readable zip archive"), "{err}");
        let err = detect_input(&write(dir.path(), "dump.sql.gz", SQL.as_bytes())).unwrap_err().to_string();
        assert!(err.contains("not gzip-compressed"), "{err}");
        let err = detect_input(&write(dir.path(), "dump.zip", SQL.as_bytes())).unwrap_err().to_string();
        assert!(err.contains("not a zip archive"), "{err}");
        let err = detect_input(&write(dir.path(), "dump.sql.xz", &[0xfd, b'7', b'z', b'X', b'Z', 0, 1, 2])).unwrap_err().to_string();
        assert!(err.contains("xz-compressed dumps are not supported"), "{err}");
        let err = detect_input(&write(dir.path(), "dump.tar.gz", &gzip(b"x"))).unwrap_err().to_string();
        assert!(err.contains("tar archives are not supported"), "{err}");
        assert!(matches!(detect_input(&dir.path().join("missing.sql.gz")), Err(PortError::NotFound { entity: "file", .. })));
    }

    #[test]
    fn a_stopped_sink_ends_decoding_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![b'x'; CHUNK * 4];
        let path = write(dir.path(), "big.sql.gz", &gzip(&big));
        let mut chunks = 0;
        let result = decode(&path, StreamCompression::Gzip, &Arc::new(AtomicU64::new(0)), |_| {
            chunks += 1;
            false
        });
        assert_eq!((result, chunks), (Ok(()), 1));
    }

    #[test]
    fn export_names_choose_the_output_compression() {
        assert_eq!(detect_output(Path::new(r"C:\b\shop.sql.gz")).unwrap(), StreamCompression::Gzip);
        assert_eq!(detect_output(Path::new(r"C:\b\shop.GZ")).unwrap(), StreamCompression::Gzip);
        assert_eq!(detect_output(Path::new(r"C:\b\shop.sql")).unwrap(), StreamCompression::None);
        assert!(detect_output(Path::new(r"C:\b\shop.zip")).unwrap_err().to_string().contains("use .sql.gz"));
    }

    #[test]
    fn gzip_encoding_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.sql.gz");
        let mut chunks = vec![b"SELECT 1;\n".to_vec(), b"SELECT 2;\n".to_vec()].into_iter();
        encode_gzip(File::create(&path).unwrap(), || chunks.next()).unwrap();
        assert_eq!(detect_input(&path).unwrap(), StreamCompression::Gzip);
        assert_eq!(decoded(&path, StreamCompression::Gzip).0.unwrap(), b"SELECT 1;\nSELECT 2;\n");
    }
}
