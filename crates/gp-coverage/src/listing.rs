//! A game's source listing with each line's PRG ROM bytes, read from the
//! debug file ld65 writes (`--dbgfile`), so executed bytes can be shown
//! over the disassembly or the game's own source.
//!
//! The debug file names the source files, each line's spans (`line`
//! records whose `type` is 0, which leaves out macro expansions), each
//! span's segment and start, and each segment's offset in the output file
//! (`ooffs`). A span's PRG ROM offset is its segment's `ooffs` plus its
//! start, less the iNES header when the output has one (a segment named
//! `HEADER` at offset 0). Spans past the end of PRG ROM, such as CHR, are
//! the caller's to drop: it knows the PRG ROM size. This assumes one output
//! file holding the header (if any) and PRG ROM, as both games' linker
//! configs write; a source file whose size differs from the one the debug
//! file records is refused, since its lines would land on the wrong bytes.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ListingError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} has changed since the debug file was written ({size} bytes, not {want}); build the listing again"
    )]
    Changed { path: PathBuf, size: u64, want: u64 },
    #[error("{path} line {line}: {message}")]
    Bad {
        path: PathBuf,
        line: usize,
        message: String,
    },
}

/// The listing: the source files with spans, and every span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub files: Vec<SourceFile>,
    /// `[file index, line number (from 1), PRG ROM offset, size]`, ordered
    /// by file and line.
    pub spans: Vec<[u64; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// The file's name, without its folder.
    pub name: String,
    pub lines: Vec<String>,
}

/// One record of the debug file: its kind and its `key=value` fields.
fn fields(rest: &str) -> HashMap<&str, &str> {
    let mut out = HashMap::new();
    let mut start = 0;
    let mut quoted = false;
    let bytes = rest.as_bytes();
    for i in 0..=bytes.len() {
        if i < bytes.len() && bytes[i] == b'"' {
            quoted = !quoted;
        }
        if i == bytes.len() || (bytes[i] == b',' && !quoted) {
            if let Some((k, v)) = rest[start..i].split_once('=') {
                out.insert(k, v.trim_matches('"'));
            }
            start = i + 1;
        }
    }
    out
}

fn number(v: &str) -> Option<u64> {
    match v.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

/// The parts of a debug file a listing needs.
#[derive(Debug, Default)]
struct Dbg {
    /// File ID to its name and size when the debug file was written.
    files: BTreeMap<u64, (String, Option<u64>)>,
    /// Segment ID to its offset in the output file.
    segs: HashMap<u64, u64>,
    header: u64,
    /// Span ID to (segment, start, size).
    spans: HashMap<u64, (u64, u64, u64)>,
    /// (file, line, span IDs).
    lines: Vec<(u64, u64, Vec<u64>)>,
}

fn parse(text: &str, path: &Path) -> Result<Dbg, ListingError> {
    let mut dbg = Dbg::default();
    for (n, line) in text.lines().enumerate() {
        let bad = |message: &str| ListingError::Bad {
            path: path.to_path_buf(),
            line: n + 1,
            message: message.into(),
        };
        let Some((kind, rest)) = line.split_once('\t') else {
            continue;
        };
        let f = fields(rest);
        let get = |k: &str| f.get(k).and_then(|v| number(v));
        match kind {
            "file" => {
                dbg.files.insert(
                    get("id").ok_or_else(|| bad("file without id"))?,
                    (
                        f.get("name")
                            .ok_or_else(|| bad("file without name"))?
                            .to_string(),
                        get("size"),
                    ),
                );
            }
            "seg" => {
                let id = get("id").ok_or_else(|| bad("seg without id"))?;
                if let Some(ooffs) = get("ooffs") {
                    dbg.segs.insert(id, ooffs);
                    if f.get("name") == Some(&"HEADER") && ooffs == 0 {
                        dbg.header = get("size").unwrap_or(0);
                    }
                }
            }
            "span" => {
                let id = get("id").ok_or_else(|| bad("span without id"))?;
                let (Some(seg), Some(start), Some(size)) = (get("seg"), get("start"), get("size"))
                else {
                    return Err(bad("span without seg, start or size"));
                };
                dbg.spans.insert(id, (seg, start, size));
            }
            "line" => {
                if f.get("type").is_some_and(|t| *t != "0") {
                    continue;
                }
                let (Some(file), Some(line), Some(spans)) =
                    (get("file"), get("line"), f.get("span"))
                else {
                    continue;
                };
                let spans = spans.split('+').filter_map(number).collect();
                dbg.lines.push((file, line, spans));
            }
            _ => {}
        }
    }
    Ok(dbg)
}

/// Reads the debug file at `path` and the source files it names.
pub fn read(path: &Path) -> Result<Listing, ListingError> {
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |source| ListingError::Io { path: p, source }
    };
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    let dbg = parse(&text, path)?;
    let mut spans = Vec::new();
    for (file, line, ids) in &dbg.lines {
        for id in ids {
            let Some(&(seg, start, size)) = dbg.spans.get(id) else {
                continue;
            };
            let Some(&ooffs) = dbg.segs.get(&seg) else {
                continue;
            };
            let Some(offset) = (ooffs + start).checked_sub(dbg.header) else {
                continue;
            };
            if ooffs == 0 && dbg.header > 0 {
                continue; // the header itself
            }
            spans.push((*file, *line, offset, size));
        }
    }
    spans.sort_unstable();
    spans.dedup();
    // Only files with spans, renumbered in order.
    let mut index = BTreeMap::new();
    let mut files = Vec::new();
    for &(file, ..) in &spans {
        if index.contains_key(&file) {
            continue;
        }
        let Some((name, size)) = dbg.files.get(&file) else {
            continue;
        };
        // Relative names are relative to where ld65 ran, which isn't
        // known; the debug file's folder is the best guess.
        let source = Path::new(name);
        let source = if source.is_absolute() {
            source.to_path_buf()
        } else {
            path.parent().unwrap_or(Path::new(".")).join(source)
        };
        let text = std::fs::read_to_string(&source).map_err(io(&source))?;
        // A source edited since the build would put its lines on the
        // wrong bytes.
        if let Some(want) = *size
            && text.len() as u64 != want
        {
            return Err(ListingError::Changed {
                path: source,
                size: text.len() as u64,
                want,
            });
        }
        index.insert(file, files.len() as u64);
        files.push(SourceFile {
            name: source
                .file_name()
                .map_or_else(|| name.clone(), |n| n.to_string_lossy().into_owned()),
            lines: text.lines().map(str::to_owned).collect(),
        });
    }
    Ok(Listing {
        files,
        spans: spans
            .into_iter()
            .filter_map(|(file, line, offset, size)| Some([*index.get(&file)?, line, offset, size]))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lines_to_prg_offsets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("game.s"), "reset:\n  sei\n  lda #$00\n").unwrap();
        let dbg = format!(
            "version\tmajor=2,minor=0\n\
             file\tid=0,name=\"{}\",size=24\n\
             seg\tid=0,name=\"CODE\",start=0x00C000,size=0x0003,type=ro,oname=\"g.nes\",ooffs=16\n\
             seg\tid=1,name=\"HEADER\",start=0x000000,size=0x0010,type=ro,oname=\"g.nes\",ooffs=0\n\
             seg\tid=2,name=\"BSS\",start=0x000000,size=0x0000,type=rw\n\
             span\tid=0,seg=0,start=0,size=1\n\
             span\tid=1,seg=0,start=1,size=2\n\
             span\tid=2,seg=1,start=0,size=4\n\
             line\tid=0,file=0,line=2,span=0\n\
             line\tid=1,file=0,line=3,span=1\n\
             line\tid=2,file=0,line=1,type=2,span=1\n\
             line\tid=3,file=0,line=1,span=2\n",
            dir.path().join("game.s").display()
        );
        let path = dir.path().join("game.dbg");
        std::fs::write(&path, dbg).unwrap();
        let listing = read(&path).unwrap();
        assert_eq!(listing.files.len(), 1);
        assert_eq!(listing.files[0].name, "game.s");
        assert_eq!(listing.files[0].lines[1], "  sei");
        assert_eq!(listing.spans, vec![[0, 2, 0, 1], [0, 3, 1, 2]]);
        // An edited source no longer matches the bytes.
        std::fs::write(
            dir.path().join("game.s"),
            "reset:\n  cli\n  sei\n  lda #$00\n",
        )
        .unwrap();
        assert!(matches!(read(&path), Err(ListingError::Changed { .. })));
    }
}
