//! Complete-line reading over plain or zstd streams. An unterminated tail is
//! never emitted, so a file mid-write is picked up on the next pass.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::roots::Format;

pub struct Line<'a> {
    pub number: i64,
    pub start_byte: i64,
    pub text: &'a str,
}

/// Opens `path` positioned at `start_byte` (plain) or at the start (zstd),
/// reading at most up to `limit` bytes of a plain file.
pub fn open(path: &Path, format: Format, start_byte: u64, limit: u64) -> io::Result<Box<dyn Read>> {
    let mut file = File::open(path)?;
    Ok(match format {
        Format::Jsonl => {
            file.seek(SeekFrom::Start(start_byte))?;
            Box::new(file.take(limit.saturating_sub(start_byte)))
        }
        Format::Zstd => Box::new(zstd::Decoder::new(file)?),
    })
}

/// Calls `f` for each complete line until it returns `Ok(false)`. Returns
/// (end byte of the last complete line, next line number).
pub fn for_each<R: Read>(
    reader: R,
    start_byte: i64,
    start_line: i64,
    mut f: impl FnMut(Line<'_>) -> io::Result<bool>,
) -> io::Result<(i64, i64)> {
    let mut reader = BufReader::with_capacity(256 * 1024, reader);
    let mut buf = Vec::new();
    let (mut pos, mut number) = (start_byte, start_line);
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            return Ok((pos, number));
        }
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        let text = String::from_utf8_lossy(&buf);
        let more = f(Line { number, start_byte: pos, text: &text })?;
        pos += n as i64;
        number += 1;
        if !more {
            return Ok((pos, number));
        }
    }
}

/// The text of line `number`, starting the search at `start_byte` (plain files
/// only; zstd always decodes from the start).
pub fn read_line(path: &Path, format: Format, number: i64, start_byte: i64) -> io::Result<Option<String>> {
    let (from_byte, from_line) = match format {
        Format::Jsonl => (start_byte.max(0) as u64, number),
        Format::Zstd => (0, 0),
    };
    let reader = open(path, format, from_byte, u64::MAX)?;
    let mut found = None;
    for_each(reader, from_byte as i64, from_line, |line| {
        if line.number == number {
            found = Some(line.text.to_string());
        }
        Ok(found.is_none())
    })?;
    Ok(found)
}
