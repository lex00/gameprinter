//! An index kept as JSON lines (one JSON value per line, appended): read
//! with the lines that don't parse reported, and one line added. Coverage's
//! and validation's caches keep theirs this way.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde::de::DeserializeOwned;

/// Line numbers (from 1) and parse errors of index lines that can't be read.
pub type BadLines = Vec<(usize, String)>;

/// Every line of the file at `path` that reads as a `T`, in order, and the
/// lines that don't. Blank lines are passed over; no file is no lines.
pub fn read_checked<T: DeserializeOwned>(path: &Path) -> io::Result<(Vec<T>, BadLines)> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e),
    };
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for (i, l) in text.lines().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(l) {
            Ok(line) => good.push(line),
            Err(e) => bad.push((i + 1, e.to_string())),
        }
    }
    Ok((good, bad))
}

/// Appends `line` (which ends in a newline) to the file at `path`, making
/// the file and its folder when they aren't there.
pub fn append(path: &Path, line: &str) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("index path has a parent"))?;
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    // One write per line, so concurrent appends can't interleave in a line.
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_append_and_read_back_with_the_bad_ones_numbered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index").join("x.jsonl");
        assert_eq!(read_checked::<u32>(&path).unwrap(), (vec![], vec![]));
        append(&path, "1\n").unwrap();
        append(&path, "\n{not json\n2\n").unwrap();
        let (good, bad) = read_checked::<u32>(&path).unwrap();
        assert_eq!(good, vec![1, 2]);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, 3);
    }
}
