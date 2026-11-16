//! Saving lines picked out of a log to a file of the user's choice.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use lampo_domain::ports::LogExportWriter;
use lampo_domain::{PortError, PortResult};

/// Line terminator of exported files: the one the logs Lampo reads are written with.
const LINE_END: &[u8] = b"\n";

/// [`LogExportWriter`] on the local file system: one buffered write of every line.
#[derive(Debug, Default, Clone, Copy)]
pub struct FileLogExportWriter;

impl LogExportWriter for FileLogExportWriter {
    fn write_lines(&self, file: &Path, lines: &[String]) -> PortResult<()> {
        let err = |e| PortError::io(format!("writing {}", file.display()), e);
        let mut out = BufWriter::new(File::create(file).map_err(err)?);
        for line in lines {
            out.write_all(line.as_bytes()).map_err(err)?;
            out.write_all(LINE_END).map_err(err)?;
        }
        out.flush().map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_one_line_each_and_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("export.log");
        std::fs::write(&file, "old content that is longer than the export\n").unwrap();

        FileLogExportWriter.write_lines(&file, &["first".to_string(), "è unicode".to_string()]).unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nè unicode\n");
    }

    #[test]
    fn no_lines_leave_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("empty.log");

        FileLogExportWriter.write_lines(&file, &[]).unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    }

    #[test]
    fn a_missing_folder_is_an_io_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("missing").join("export.log");

        let err = FileLogExportWriter.write_lines(&file, &["x".to_string()]).unwrap_err();

        assert!(matches!(&err, PortError::Io { context, .. } if context == &format!("writing {}", file.display())), "{err:?}");
    }
}
