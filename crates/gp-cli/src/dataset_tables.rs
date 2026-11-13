//! The tables a data set is written as (game-printer's serve/dataset.rs
//! and serve/dataset_hf.rs, the parts `gp dataset rom` uses): typed
//! columns, deterministic Parquet, Croissant metadata, and the check that
//! no file written holds a run of the ROM's bytes. `gp dataset export`
//! (wave 9 of the port) can share them.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{Value, json};

/// A column of one table.
pub(crate) enum Col {
    /// Text, null where `None`.
    Str(Vec<Option<String>>),
    Int(Vec<i64>),
    /// A whole number, null where `None`.
    OptInt(Vec<Option<i64>>),
    Bool(Vec<bool>),
    /// A float for plotting, null where the value is a name. The exact
    /// value is the text column beside it.
    Num(Vec<Option<f64>>),
}

impl Col {
    fn len(&self) -> usize {
        match self {
            Col::Str(v) => v.len(),
            Col::Int(v) => v.len(),
            Col::OptInt(v) => v.len(),
            Col::Bool(v) => v.len(),
            Col::Num(v) => v.len(),
        }
    }

    pub(crate) fn parquet_type(&self) -> &'static str {
        match self {
            Col::Str(_) => "OPTIONAL BYTE_ARRAY",
            Col::Int(_) => "REQUIRED INT64",
            Col::OptInt(_) => "OPTIONAL INT64",
            Col::Bool(_) => "REQUIRED BOOLEAN",
            Col::Num(_) => "OPTIONAL DOUBLE",
        }
    }

    pub(crate) fn croissant_type(&self) -> &'static str {
        match self {
            Col::Str(_) => "sc:Text",
            Col::Int(_) | Col::OptInt(_) => "sc:Integer",
            Col::Bool(_) => "sc:Boolean",
            Col::Num(_) => "sc:Float",
        }
    }
}

/// One column's name, unit and meaning, for the card and Croissant.
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) about: &'static str,
    pub(crate) col: Col,
}

pub(crate) struct Table {
    pub(crate) name: &'static str,
    pub(crate) about: &'static str,
    /// The columns that name a row.
    pub(crate) key: &'static [&'static str],
    pub(crate) fields: Vec<Field>,
}

impl Table {
    pub(crate) fn rows(&self) -> usize {
        self.fields.first().map_or(0, |f| f.col.len())
    }
}

/// Writes `t` as one Parquet file with one row group, uncompressed, and
/// the format, units and key in its metadata, so the same rows give the
/// same bytes.
pub(crate) fn write_parquet(
    t: &Table,
    path: &Path,
    meta: &[(String, String)],
) -> anyhow::Result<()> {
    use parquet::data_type::{BoolType, ByteArray, ByteArrayType, DoubleType, Int64Type};
    use parquet::file::metadata::KeyValue;
    use parquet::file::properties::WriterProperties;
    use parquet::file::writer::SerializedFileWriter;

    let mut schema = format!("message {} {{\n", t.name);
    for f in &t.fields {
        let logical = if matches!(f.col, Col::Str(_)) {
            " (STRING)"
        } else {
            ""
        };
        let _ = writeln!(schema, "  {} {}{logical};", f.col.parquet_type(), f.name);
    }
    schema.push('}');
    let schema = Arc::new(parquet::schema::parser::parse_message_type(&schema)?);
    let units: serde_json::Map<String, Value> = t
        .fields
        .iter()
        .filter(|f| !f.unit.is_empty())
        .map(|f| (f.name.to_owned(), json!(f.unit)))
        .collect();
    let mut kv: Vec<KeyValue> = meta
        .iter()
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .collect();
    kv.push(KeyValue::new("gp.table".into(), t.name.to_owned()));
    kv.push(KeyValue::new("gp.key".into(), t.key.join(",")));
    kv.push(KeyValue::new(
        "gp.units".into(),
        Value::Object(units).to_string(),
    ));
    let props = WriterProperties::builder()
        .set_created_by(format!("gp {}", env!("CARGO_PKG_VERSION")))
        .set_key_value_metadata(Some(kv))
        .build();
    let file = fs::File::create(path).with_context(|| format!("writing {}", path.display()))?;
    let mut w = SerializedFileWriter::new(file, schema, Arc::new(props))?;
    if t.rows() == 0 {
        // No row group at all: readers that size batches by a row
        // group's rows (Hugging Face `datasets`) refuse an empty one.
        w.close()?;
        return Ok(());
    }
    let mut rg = w.next_row_group()?;
    let mut k = 0;
    while let Some(mut cw) = rg.next_column()? {
        match &t.fields[k].col {
            Col::Str(v) => {
                let vals: Vec<ByteArray> = v.iter().flatten().map(|x| x.as_str().into()).collect();
                let defs: Vec<i16> = v.iter().map(|x| i16::from(x.is_some())).collect();
                cw.typed::<ByteArrayType>()
                    .write_batch(&vals, Some(&defs), None)?;
            }
            Col::Int(v) => {
                cw.typed::<Int64Type>().write_batch(v, None, None)?;
            }
            Col::OptInt(v) => {
                let vals: Vec<i64> = v.iter().flatten().copied().collect();
                let defs: Vec<i16> = v.iter().map(|x| i16::from(x.is_some())).collect();
                cw.typed::<Int64Type>()
                    .write_batch(&vals, Some(&defs), None)?;
            }
            Col::Bool(v) => {
                cw.typed::<BoolType>().write_batch(v, None, None)?;
            }
            Col::Num(v) => {
                let vals: Vec<f64> = v.iter().flatten().copied().collect();
                let defs: Vec<i16> = v.iter().map(|x| i16::from(x.is_some())).collect();
                cw.typed::<DoubleType>()
                    .write_batch(&vals, Some(&defs), None)?;
            }
        }
        cw.close()?;
        k += 1;
    }
    rg.close()?;
    w.close()?;
    Ok(())
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Croissant 1.0 metadata for any set of tables: one FileObject and one
/// RecordSet a table, each file at `data/<table>.<ext>`.
pub(crate) fn croissant_for(
    name: &str,
    description: &str,
    version: &str,
    tables: &[Table],
    ext: &str,
    files: &[(String, String, usize)],
) -> Value {
    let format = if ext == "parquet" {
        "application/x-parquet"
    } else {
        "text/csv"
    };
    let dist: Vec<Value> = tables
        .iter()
        .map(|t| {
            let f = format!("data/{}.{ext}", t.name);
            let h = files.iter().find(|x| x.0 == f).map_or("", |x| x.1.as_str());
            json!({"@type": "cr:FileObject", "@id": t.name, "name": t.name,
                   "contentUrl": f, "encodingFormat": format, "sha256": h})
        })
        .collect();
    let sets: Vec<Value> = tables
        .iter()
        .map(|t| {
            let fields: Vec<Value> = t
                .fields
                .iter()
                .map(|f| {
                    json!({"@type": "cr:Field", "@id": format!("{}/{}", t.name, f.name),
                       "name": f.name, "description": f.about, "dataType": f.col.croissant_type(),
                       "source": {"fileObject": {"@id": t.name}, "extract": {"column": f.name}}})
                })
                .collect();
            let key: Vec<Value> = t
                .key
                .iter()
                .map(|k| json!({"@id": format!("{}/{k}", t.name)}))
                .collect();
            json!({"@type": "cr:RecordSet", "@id": t.name, "name": t.name,
                   "description": t.about, "key": key, "field": fields})
        })
        .collect();
    json!({
        "@context": {"@language": "en", "@vocab": "https://schema.org/", "cr": "http://mlcommons.org/croissant/",
                     "dct": "http://purl.org/dc/terms/", "sc": "https://schema.org/",
                     "conformsTo": "dct:conformsTo", "recordSet": "cr:recordSet", "field": "cr:field",
                     "fileObject": "cr:fileObject", "source": "cr:source", "extract": "cr:extract",
                     "column": "cr:column", "dataType": {"@id": "cr:dataType", "@type": "@vocab"},
                     "key": "cr:key"},
        "@type": "sc:Dataset",
        "conformsTo": "http://mlcommons.org/croissant/1.0",
        "name": name,
        "description": description,
        "license": "https://github.com/lex00/gameprinter/blob/main/docs/research/datasets.md",
        "url": "https://github.com/lex00/gameprinter",
        "creator": {"@type": "sc:Person", "name": "Game Printer"},
        "datePublished": "1970-01-01",
        "version": version,
        "distribution": dist,
        "recordSet": sets,
    })
}

/// The length of a run of ROM bytes that counts as a copy.
pub(crate) const WINDOW: usize = 16;

/// Windows of 16 bytes too regular to say where they came from, as
/// scripts/romguard/rom_text.py skips them: fewer than 4 values, a
/// pattern of 4 bytes or fewer repeated, or a ramp of one step.
pub(crate) fn trivial(w: &[u8]) -> bool {
    let distinct: BTreeSet<u8> = w.iter().copied().collect();
    if distinct.len() < 4 {
        return true;
    }
    // Mostly zeros: what small numbers in 64-bit columns look like
    // (0x0102 then 0x0103, a RAM address column), and too little of a ROM
    // to matter when the ROM has the same bytes by chance.
    if w.iter().filter(|&&b| b == 0).count() >= 10 {
        return true;
    }
    if (1..=4).any(|p| (p..w.len()).all(|k| w[k] == w[k - p])) {
        return true;
    }
    let step = w[1].wrapping_sub(w[0]);
    w.windows(2).all(|x| x[1].wrapping_sub(x[0]) == step)
}

/// Every file under `dir` that holds a run of `WINDOW` bytes of `rom`
/// (headerless PRG and CHR), with the offset of the first such run: the
/// files as written, so a ROM table copied into a Parquet string, the
/// card or Croissant is found. Trivial windows are skipped.
pub(crate) fn rom_runs(dir: &Path, rom: &[u8]) -> anyhow::Result<Vec<(String, usize)>> {
    let windows: HashSet<&[u8]> = rom.windows(WINDOW).filter(|w| !trivial(w)).collect();
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files.sort();
    for p in files {
        let bytes = fs::read(&p)?;
        if let Some(at) = bytes.windows(WINDOW).position(|w| windows.contains(w)) {
            let rel = p.strip_prefix(dir).unwrap_or(&p).display().to_string();
            found.push((rel, at));
        }
    }
    Ok(found)
}
