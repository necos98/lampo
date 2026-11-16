//! The input of a piped command as its first stage receives it: the file decoded by
//! [`crate::compression`], then rewritten by a [`DumpRewriter`] (dumps from other servers), chunk
//! by chunk. Nothing is written to disk and memory stays bounded.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use lampo_domain::ports::StreamCompression;
use lampo_domain::sql_dump::rewrite::{DumpRewrite, DumpRewriter};

use crate::compression;

/// Streams `path`, decoded and rewritten, into `sink` (blocking: run it on a blocking thread).
/// Same contract as [`compression::decode`]: `sink` returning `false` stops quietly, `Err` is a
/// message for corrupt or unreadable data, `position` = bytes of the file consumed.
pub(crate) fn read(path: &Path, compression: StreamCompression, rewrite: DumpRewrite, position: &Arc<AtomicU64>, mut sink: impl FnMut(Vec<u8>) -> bool) -> Result<(), String> {
    if rewrite.is_noop() {
        return compression::decode(path, compression, position, sink);
    }
    let mut rewriter = DumpRewriter::new(rewrite);
    let mut accepting = true;
    compression::decode(path, compression, position, |chunk| {
        let mut out = Vec::with_capacity(chunk.len());
        rewriter.push(&chunk, &mut out);
        accepting = out.is_empty() || sink(out);
        accepting
    })?;
    if accepting {
        let mut out = Vec::new();
        rewriter.finish(&mut out);
        if !out.is_empty() {
            sink(out);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    const MARIADB_DUMP: &str = "/*M!999999\\- enable the sandbox mode */\n\
        -- MariaDB dump 10.19\n\
        INSERT INTO `notes` VALUES (1,'CREATE DEFINER=`x`@`y` PROCEDURE'),(2,'/*!50013 DEFINER=`x`@`y` */');\n\
        /*!50001 CREATE ALGORITHM=UNDEFINED */\n\
        /*!50013 DEFINER=`app`@`localhost` SQL SECURITY DEFINER */\n\
        /*!50001 VIEW `v` AS select 1 AS `x` */;\n\
        CREATE DEFINER=`app`@`localhost` PROCEDURE `p`()\nBEGIN\nEND ;;\n\
        /*!50003 CREATE*/ /*!50017 DEFINER=`app`@`localhost`*/ /*!50003 TRIGGER `t` BEFORE INSERT ON `notes` FOR EACH ROW SET NEW.id = 1 */;;\n";

    const REWRITTEN: &str = "-- MariaDB dump 10.19\n\
        INSERT INTO `notes` VALUES (1,'CREATE DEFINER=`x`@`y` PROCEDURE'),(2,'/*!50013 DEFINER=`x`@`y` */');\n\
        /*!50001 CREATE ALGORITHM=UNDEFINED */\n\
        /*!50013 SQL SECURITY DEFINER */\n\
        /*!50001 VIEW `v` AS select 1 AS `x` */;\n\
        CREATE PROCEDURE `p`()\nBEGIN\nEND ;;\n\
        /*!50003 CREATE*/ /*!50017 */ /*!50003 TRIGGER `t` BEFORE INSERT ON `notes` FOR EACH ROW SET NEW.id = 1 */;;\n";

    const ALL: DumpRewrite = DumpRewrite { drop_sandbox_line: true, strip_definers: true };

    fn read_all(path: &Path, compression: StreamCompression, rewrite: DumpRewrite) -> String {
        let mut out = Vec::new();
        read(path, compression, rewrite, &Arc::new(AtomicU64::new(0)), |chunk| {
            out.extend(chunk);
            true
        })
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    fn gzip_file(dir: &Path, content: &[u8]) -> PathBuf {
        let path = dir.join("dump.sql.gz");
        let mut e = flate2::write::GzEncoder::new(std::fs::File::create(&path).unwrap(), flate2::Compression::fast());
        e.write_all(content).unwrap();
        e.finish().unwrap();
        path
    }

    fn zip_file(dir: &Path, content: &[u8]) -> PathBuf {
        let path = dir.join("dump.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        w.start_file("shop.sql", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(content).unwrap();
        w.finish().unwrap();
        path
    }

    #[test]
    fn plain_gzip_and_zip_dumps_are_rewritten_on_the_way_and_left_unchanged_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("dump.sql");
        std::fs::write(&plain, MARIADB_DUMP).unwrap();
        let inputs = [
            (plain, StreamCompression::None),
            (gzip_file(dir.path(), MARIADB_DUMP.as_bytes()), StreamCompression::Gzip),
            (zip_file(dir.path(), MARIADB_DUMP.as_bytes()), StreamCompression::Zip),
        ];
        for (path, compression) in &inputs {
            let before = std::fs::read(path).unwrap();
            assert_eq!(read_all(path, *compression, ALL), REWRITTEN, "{compression:?}");
            assert_eq!(std::fs::read(path).unwrap(), before, "the source file is never touched");
        }
        assert_eq!(read_all(&inputs[1].0, StreamCompression::Gzip, DumpRewrite::default()), MARIADB_DUMP, "no rewrite: decoded as it is");
    }

    #[test]
    fn a_long_dump_streams_in_bounded_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let row = format!("INSERT INTO t VALUES ('{}');\n", "x".repeat(200_000));
        let dump = format!("CREATE DEFINER=`a`@`b` VIEW v AS SELECT 1\n{row}{row}");
        let path = gzip_file(dir.path(), dump.as_bytes());
        let mut largest = 0;
        let mut out = Vec::new();
        read(&path, StreamCompression::Gzip, ALL, &Arc::new(AtomicU64::new(0)), |chunk| {
            largest = largest.max(chunk.len());
            out.extend(chunk);
            true
        })
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), format!("CREATE VIEW v AS SELECT 1\n{row}{row}"));
        assert!(largest <= compression::CHUNK + lampo_domain::sql_dump::rewrite::LINE_HEAD_BYTES, "{largest}");
    }

    #[test]
    fn a_last_line_without_line_break_is_flushed_and_a_stopped_sink_ends_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dump.sql");
        std::fs::write(&path, "CREATE DEFINER=`a`@`b` VIEW v AS SELECT 1").unwrap();
        assert_eq!(read_all(&path, StreamCompression::None, ALL), "CREATE VIEW v AS SELECT 1");

        std::fs::write(&path, "SELECT 1;\n".repeat(compression::CHUNK)).unwrap();
        let mut chunks = 0;
        let result = read(&path, StreamCompression::None, ALL, &Arc::new(AtomicU64::new(0)), |_| {
            chunks += 1;
            false
        });
        assert_eq!((result, chunks), (Ok(()), 1));
    }
}
