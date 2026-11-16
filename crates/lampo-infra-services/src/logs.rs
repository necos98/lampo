use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use lampo_domain::logs::{LogChunk, LogLine, LogQuery};
use lampo_domain::ports::LogFileReader;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::log_rotation::current_log_file;

/// Block size for reading backwards.
const BLOCK: u64 = 64 * 1024;
/// Most bytes returned by one `read_from` (`--follow` catches up over several polls).
const MAX_FOLLOW_READ: u64 = 1024 * 1024;

/// Reads log files backwards for `tail` (no full read of big logs); parses timestamps of the
/// Apache error/access, PHP error and MySQL error formats for `since`.
///
/// Only complete (newline-terminated) lines are returned, so `next_offset` is the end of the
/// last complete line: the end of file in the usual case, and the start of a line still being
/// written otherwise (so `--follow` never returns half a line).
///
/// A rotated log is read from the member of its set being written (`current_log_file`): Apache
/// logs go round `error.log`, `error.log.1`, `error.log.2`, and right after a rotation the
/// previous file is the newest one. When `--follow` moves to a new file, that file is shorter
/// than the offset and `read_from` restarts from its beginning.
#[derive(Debug, Default, Clone, Copy)]
pub struct FileLogReader;

fn open(file: &Path) -> PortResult<Option<File>> {
    match File::open(file) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PortError::io(format!("opening {}", file.display()), e)),
    }
}

fn read_at(f: &mut File, start: u64, len: u64) -> io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len as usize);
    f.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

fn to_line(bytes: &[u8]) -> LogLine {
    let text = String::from_utf8_lossy(bytes).trim_end_matches('\r').to_string();
    let at_unix_ms = parse_timestamp(&text);
    LogLine { text, at_unix_ms }
}

/// Complete lines of a file, newest first, read block by block from the end.
struct ReverseLines {
    file: File,
    /// File offset of `buf[0]`.
    pos: u64,
    /// Unemitted bytes `[pos, …)`; when non-empty it ends with `\n`.
    buf: Vec<u8>,
    /// End of the last complete line.
    end: u64,
}

impl ReverseLines {
    fn new(mut file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        // Skip a trailing line still being written (no newline yet).
        let mut pos = len;
        while pos > 0 {
            let start = pos.saturating_sub(BLOCK);
            let mut chunk = read_at(&mut file, start, pos - start)?;
            if let Some(i) = chunk.iter().rposition(|&b| b == b'\n') {
                chunk.truncate(i + 1);
                let end = start + i as u64 + 1;
                return Ok(Self { file, pos: start, buf: chunk, end });
            }
            pos = start;
        }
        Ok(Self { file, pos: 0, buf: Vec::new(), end: 0 })
    }

    fn next_line(&mut self) -> io::Result<Option<LogLine>> {
        loop {
            if self.buf.is_empty() {
                return Ok(None);
            }
            // Index of the newline terminating the line to emit.
            let nl = self.buf.len() - 1;
            if let Some(i) = self.buf[..nl].iter().rposition(|&b| b == b'\n') {
                let line = to_line(&self.buf[i + 1..nl]);
                self.buf.truncate(i + 1);
                return Ok(Some(line));
            }
            if self.pos == 0 {
                let line = to_line(&self.buf[..nl]);
                self.buf.clear();
                return Ok(Some(line));
            }
            let start = self.pos.saturating_sub(BLOCK);
            let mut chunk = read_at(&mut self.file, start, self.pos - start)?;
            chunk.append(&mut self.buf);
            self.buf = chunk;
            self.pos = start;
        }
    }
}

impl LogFileReader for FileLogReader {
    fn tail(&self, file: &Path, query: &LogQuery) -> PortResult<LogChunk> {
        let file = &current_log_file(file);
        let Some(f) = open(file)? else { return Ok(LogChunk::default()) };
        let err = |e| PortError::io(format!("reading {}", file.display()), e);
        let mut lines = ReverseLines::new(f).map_err(err)?;
        let next_offset = lines.end;
        let mut out = Vec::new();
        while out.len() < query.max_lines {
            let Some(line) = lines.next_line().map_err(err)? else { break };
            if let (Some(since), Some(at)) = (query.since_unix_ms, line.at_unix_ms) {
                if at < since {
                    // Logs are chronological: everything earlier is older too.
                    break;
                }
            }
            out.push(line);
        }
        out.reverse();
        Ok(LogChunk { lines: out, next_offset })
    }

    fn read_from(&self, file: &Path, offset: u64) -> PortResult<LogChunk> {
        let file = &current_log_file(file);
        let Some(mut f) = open(file)? else { return Ok(LogChunk::default()) };
        let err = |e| PortError::io(format!("reading {}", file.display()), e);
        let len = f.metadata().map_err(err)?.len();
        let offset = if len < offset { 0 } else { offset };
        let want = (len - offset).min(MAX_FOLLOW_READ);
        let data = read_at(&mut f, offset, want).map_err(err)?;
        let consumed = match data.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            // One huge unterminated line filling the whole window: return it rather than stall.
            None if want == MAX_FOLLOW_READ => data.len(),
            None => 0,
        };
        let body = &data[..consumed];
        let body = body.strip_suffix(b"\n").unwrap_or(body);
        let lines = if consumed == 0 { Vec::new() } else { body.split(|&b| b == b'\n').map(to_line).collect() };
        Ok(LogChunk { lines, next_offset: offset + consumed as u64 })
    }
}

// ---------------------------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------------------------

/// Days-from-civil (proleptic Gregorian) to Unix milliseconds.
pub(crate) fn civil_to_unix_ms(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32, millis: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 || millis > 999 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    u64::try_from(secs * 1000 + millis as i64).ok()
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let lower = name.get(..3)?.to_ascii_lowercase();
    MONTHS.iter().position(|m| *m == lower).map(|i| i as u32 + 1)
}

fn num(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `HH:MM:SS` with optional `.fraction`; returns (h, m, s, millis).
fn parse_clock(s: &str) -> Option<(u32, u32, u32, u32)> {
    let (hms, frac) = s.split_once('.').map_or((s, ""), |(a, b)| (a, b));
    let mut it = hms.split(':');
    let (h, m, sec) = (num(it.next()?)?, num(it.next()?)?, num(it.next()?)?);
    if it.next().is_some() {
        return None;
    }
    let millis = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).take(3).collect();
        format!("{digits:0<3}").parse().ok()?
    };
    Some((h, m, sec, millis))
}

fn local_ms(y: u32, mo: u32, d: u32, (h, mi, s, ms): (u32, u32, u32, u32)) -> Option<u64> {
    crate::win::local_to_unix_ms(y as u16, mo as u16, d as u16, h as u16, mi as u16, s as u16, ms as u16)
}

/// ISO 8601 at the start of the line: `2026-11-13T10:15:30.123456Z`, `…+01:00`, or with a space
/// and no zone (local time). MySQL error logs and Lampo's captured service output.
fn parse_iso(line: &str) -> Option<u64> {
    let date = line.get(..10)?;
    let (y, mo, d) = (num(date.get(..4)?)?, num(date.get(5..7)?)?, num(date.get(8..10)?)?);
    if date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return None;
    }
    let sep = *line.as_bytes().get(10)?;
    if sep != b'T' && sep != b' ' {
        return None;
    }
    let rest = &line[11..];
    let clock_len = rest.find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.')).unwrap_or(rest.len());
    let clock = parse_clock(&rest[..clock_len])?;
    let zone = &rest[clock_len..];
    let utc = civil_to_unix_ms(y as i64, mo, d, clock.0, clock.1, clock.2, clock.3)?;
    if zone.starts_with('Z') {
        return Some(utc);
    }
    if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
        let z = zone[1..].get(..5).or_else(|| zone[1..].get(..4))?;
        let (zh, zm) = if z.as_bytes().get(2) == Some(&b':') { (num(&z[..2])?, num(&z[3..5])?) } else { (num(&z[..2])?, num(&z[2..4])?) };
        let offset = (zh as u64 * 60 + zm as u64) * 60_000;
        return Some(if sign == '+' { utc.checked_sub(offset)? } else { utc + offset });
    }
    local_ms(y, mo, d, clock)
}

/// Apache error log: `[Wed Oct 11 14:32:52.123456 2000] …` (local time).
fn parse_apache_error(line: &str) -> Option<u64> {
    let inner = line.strip_prefix('[')?.split(']').next()?;
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let mo = month_number(parts[1])?;
    let d = num(parts[2])?;
    let clock = parse_clock(parts[3])?;
    let y = num(parts[4])?;
    local_ms(y, mo, d, clock)
}

/// PHP error log: `[11-Nov-2026 14:32:52 UTC] …` (`UTC`, otherwise treated as local time).
fn parse_php(line: &str) -> Option<u64> {
    let inner = line.strip_prefix('[')?.split(']').next()?;
    let mut parts = inner.split_whitespace();
    let date = parts.next()?;
    let clock = parse_clock(parts.next()?)?;
    let zone = parts.next().unwrap_or("");
    let mut dp = date.split('-');
    let (d, mo, y) = (num(dp.next()?)?, month_number(dp.next()?)?, num(dp.next()?)?);
    if zone.eq_ignore_ascii_case("UTC") { civil_to_unix_ms(y as i64, mo, d, clock.0, clock.1, clock.2, clock.3) } else { local_ms(y, mo, d, clock) }
}

/// Apache access log (common/combined): `… [10/Oct/2000:13:55:36 -0700] "GET …`.
fn parse_access(line: &str) -> Option<u64> {
    let start = line.find('[')?;
    let inner = line[start + 1..].split(']').next()?;
    let (stamp, zone) = inner.split_once(' ')?;
    let mut it = stamp.splitn(4, ['/', ':']);
    let (d, mo, y) = (num(it.next()?)?, month_number(it.next()?)?, num(it.next()?)?);
    let clock = parse_clock(it.next()?)?;
    let utc = civil_to_unix_ms(y as i64, mo, d, clock.0, clock.1, clock.2, clock.3)?;
    let sign = zone.chars().next()?;
    let (zh, zm) = (num(zone.get(1..3)?)?, num(zone.get(3..5)?)?);
    let offset = (zh as u64 * 60 + zm as u64) * 60_000;
    match sign {
        '+' => utc.checked_sub(offset),
        '-' => Some(utc + offset),
        _ => None,
    }
}

/// Parses the leading timestamp of the supported log formats.
pub(crate) fn parse_timestamp(line: &str) -> Option<u64> {
    if line.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return parse_iso(line).or_else(|| parse_access(line));
    }
    if line.starts_with('[') {
        return parse_apache_error(line).or_else(|| parse_php(line));
    }
    parse_access(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_to_unix_ms(1970, 1, 1, 0, 0, 0, 0), Some(0));
        assert_eq!(civil_to_unix_ms(2000, 2, 29, 12, 0, 0, 5), Some(951_825_600_005));
        assert_eq!(civil_to_unix_ms(2026, 13, 1, 0, 0, 0, 0), None);
    }

    #[test]
    fn parses_known_formats() {
        let utc = civil_to_unix_ms(2026, 11, 13, 10, 15, 30, 123).unwrap();
        assert_eq!(parse_timestamp("2026-11-13T10:15:30.123456Z 0 [System] [MY-010116] [Server] ready"), Some(utc));
        assert_eq!(parse_timestamp("2026-11-13T11:15:30.123+01:00 0 [Note] x"), Some(utc));
        assert_eq!(parse_timestamp("[13-Nov-2026 10:15:30 UTC] PHP Warning: x"), Some(utc - 123));
        assert_eq!(parse_timestamp(r#"127.0.0.1 - - [13/Nov/2026:11:15:30 +0100] "GET / HTTP/1.1" 200 5"#), Some(utc - 123));
        // Local-time formats parse (exact value depends on the machine's time zone).
        assert!(parse_timestamp("[Fri Nov 13 10:15:30.123456 2026] [core:error] [pid 1] x").is_some());
        assert!(parse_timestamp("2026-11-13 10:15:30 x").is_some());
        assert_eq!(parse_timestamp("Stack trace:"), None);
        assert_eq!(parse_timestamp("#0 {main}"), None);
    }

    fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::File::create(&p).unwrap().write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let r = FileLogReader;
        assert_eq!(r.tail(&dir.path().join("none.log"), &LogQuery { max_lines: 5, since_unix_ms: None }).unwrap(), LogChunk::default());
        assert_eq!(r.read_from(&dir.path().join("none.log"), 10).unwrap(), LogChunk::default());
    }

    #[test]
    fn tail_reads_backwards_across_blocks_and_skips_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut content = String::new();
        for i in 0..20_000 {
            content.push_str(&format!("line {i:05} {}\r\n", "x".repeat(i % 50)));
        }
        let complete = content.len() as u64;
        content.push_str("partial");
        let p = write(dir.path(), "a.log", &content);
        let chunk = FileLogReader.tail(&p, &LogQuery { max_lines: 3, since_unix_ms: None }).unwrap();
        let texts: Vec<_> = chunk.lines.iter().map(|l| l.text.split(' ').take(2).collect::<Vec<_>>().join(" ")).collect();
        assert_eq!(texts, ["line 19997", "line 19998", "line 19999"]);
        assert_eq!(chunk.next_offset, complete);
        let all = FileLogReader.tail(&p, &LogQuery { max_lines: 100_000, since_unix_ms: None }).unwrap();
        assert_eq!(all.lines.len(), 20_000);
        assert_eq!(all.lines[0].text, "line 00000 ");
    }

    #[test]
    fn tail_since_stops_at_older_entries_and_keeps_untimed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "e.log", "2026-11-13T10:00:00Z 0 [ERROR] old\n2026-11-13T10:05:00Z 0 [ERROR] new\ncontinuation\n2026-11-13T10:06:00Z 0 [Note] newer\n");
        let since = civil_to_unix_ms(2026, 11, 13, 10, 1, 0, 0);
        let chunk = FileLogReader.tail(&p, &LogQuery { max_lines: 10, since_unix_ms: since }).unwrap();
        let texts: Vec<_> = chunk.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["2026-11-13T10:05:00Z 0 [ERROR] new", "continuation", "2026-11-13T10:06:00Z 0 [Note] newer"]);
        assert_eq!(chunk.lines[1].at_unix_ms, None);
    }

    #[test]
    fn reads_the_member_of_a_rotation_set_being_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "access.log", "old 1\nold 2\n");
        let first = FileLogReader.tail(&p, &LogQuery { max_lines: 10, since_unix_ms: None }).unwrap();
        // rotatelogs moved on to `access.log.1`: the base file is now the older one.
        let older = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(older).unwrap();
        write(dir.path(), "access.log.1", "new\n");
        let chunk = FileLogReader.tail(&p, &LogQuery { max_lines: 10, since_unix_ms: None }).unwrap();
        assert_eq!(chunk.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["new"]);
        // `--follow` from the old file's offset restarts at the top of the new one.
        let followed = FileLogReader.read_from(&p, first.next_offset).unwrap();
        assert_eq!(followed.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["new"]);
        assert_eq!(followed.next_offset, 4);
    }

    #[test]
    fn read_from_returns_complete_lines_and_restarts_after_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "f.log", "one\ntwo\nthr");
        let c = FileLogReader.read_from(&p, 0).unwrap();
        assert_eq!(c.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(c.next_offset, 8);
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"ee\n").unwrap();
        let c2 = FileLogReader.read_from(&p, c.next_offset).unwrap();
        assert_eq!(c2.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["three"]);
        let none = FileLogReader.read_from(&p, c2.next_offset).unwrap();
        assert!(none.lines.is_empty());
        assert_eq!(none.next_offset, c2.next_offset);
        // Rotated: shorter than the offset → from the start.
        write(dir.path(), "f.log", "new\n");
        let c3 = FileLogReader.read_from(&p, c2.next_offset).unwrap();
        assert_eq!(c3.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["new"]);
        assert_eq!(c3.next_offset, 4);
    }
}
