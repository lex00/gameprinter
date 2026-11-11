//! Junk around a dump in the `gp roms` commands: the findings `gp roms scan`
//! reports, `gp roms clean`, and `gp roms add --clean`. The detection and
//! cleaning are gp_evidence::dump. Brought over from game-printer, where
//! docs/sources.md, "Junk around a dump", describes them; the registry is
//! now every game's roms.toml in the games checkout (`roms_cmd::sources`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::dump::{self, Cleaned, DumpError};
use gp_evidence::registry::{KnownSource, Registry};
use gp_evidence::source::{NesImage, SourceError, identify_nes};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Outcome;

/// The GoodNES tags that hint a dump was changed, named in reports.
const MODIFIED_HINT: &str =
    "GoodNES tags such as [h], [p], [T], [t], [o], [b] in the file name are a hint";

/// What `gp roms scan` says about one file beyond its identity: each
/// finding, header contradictions with the registry, and, for a file that
/// matches no source, whether it may be a modified copy of one. JSON and
/// text lines.
pub(crate) fn scan_findings(
    bytes: &[u8],
    file_name: &str,
    registry: &Registry,
) -> Result<(Value, Vec<String>), SourceError> {
    let (img, findings) = dump::inspect(bytes)?;
    let mut lines: Vec<String> = findings.iter().map(dump::Finding::line).collect();
    let known = registry.by_sha256(&img.sha256);
    let contradictions = known
        .and_then(KnownSource::board_facts)
        .map(|f| dump::contradictions(&img.header, &f))
        .unwrap_or_default();
    if !contradictions.is_empty() {
        lines.push(format!(
            "header: contradicts the registry ({}); gp-nes refuses this file, `gp roms clean` writes a copy with the registry's header",
            join(&contradictions)
        ));
    }
    let tags = dump::name_tags(file_name);
    let release = release(&img, known, registry);
    if let Some(line) = &release.line {
        lines.push(line.clone());
    }
    if !tags.is_empty() {
        lines.push(format!(
            "file name tags (hints only): {}",
            tags.iter()
                .map(|t| format!("{} {}", t.tag, t.meaning))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok((
        json!({
            "findings": findings,
            "contradictions": contradictions,
            "release": release,
            "name_tags": tags,
        }),
        lines,
    ))
}

fn join(c: &[dump::Contradiction]) -> String {
    c.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Whether the cleaned data is a known release, and if not, which sources
/// have the same PRG and CHR sizes.
#[derive(Debug, Serialize)]
struct Release {
    known: bool,
    /// Sources whose board facts give the same mapper and PRG and CHR sizes.
    same_sizes_as: Vec<String>,
    #[serde(skip)]
    line: Option<String>,
}

fn release(img: &NesImage, known: Option<&KnownSource>, registry: &Registry) -> Release {
    if known.is_some() {
        return Release {
            known: true,
            same_sizes_as: Vec::new(),
            line: None,
        };
    }
    let same: Vec<String> = registry
        .sources
        .iter()
        .filter(|s| {
            s.board_facts().is_some_and(|f| {
                f.mapper == img.header.mapper
                    && f.prg_rom_bytes == img.header.prg_rom_bytes
                    && f.chr_rom_bytes == img.header.chr_rom_bytes
            })
        })
        .map(|s| s.id.clone())
        .collect();
    let line = if same.is_empty() {
        "release: matches no known release".to_owned()
    } else {
        format!(
            "release: matches no known release; same mapper and PRG and CHR sizes as {}, so it may be a modified copy ({MODIFIED_HINT})",
            same.join(", ")
        )
    };
    Release {
        known: false,
        same_sizes_as: same,
        line: Some(line),
    }
}

/// The underdump message for a file shorter than its header says.
pub(crate) fn problem_line(e: &SourceError) -> String {
    match e {
        SourceError::Truncated { declared, actual } => format!(
            "underdump: the header states {} bytes of PRG and CHR, the file has {} after the header; not fixed",
            dump::thousands(*declared),
            dump::thousands(*actual)
        ),
        other => other.to_string(),
    }
}

/// The sidecar written next to a cleaned copy, `<out>.clean.toml`.
#[derive(Debug, Serialize)]
struct Sidecar<'a> {
    schema: u32,
    tool: &'static str,
    input: &'a Path,
    input_bytes: u64,
    input_sha256: &'a str,
    output: &'a Path,
    output_bytes: u64,
    /// Headerless SHA-256, the same for input and output.
    sha256: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    header_rule: dump::HeaderRule,
    header: String,
    contradictions: Vec<String>,
    findings: Vec<String>,
    removed: Vec<dump::Removed>,
}

pub(crate) fn sidecar_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".clean.toml");
    out.with_file_name(name)
}

/// One cleaned file, ready to write.
pub(crate) struct CleanJob {
    pub image: NesImage,
    pub cleaned: Cleaned,
    pub source: Option<String>,
}

pub(crate) fn prepare(bytes: &[u8], registry: &Registry) -> Result<CleanJob, DumpError> {
    let image = identify_nes(bytes)?;
    let known = registry.by_sha256(&image.sha256);
    let facts = known.and_then(KnownSource::board_facts);
    let cleaned = dump::clean(bytes, facts.as_ref())?;
    Ok(CleanJob {
        image,
        cleaned,
        source: known.map(|k| k.id.clone()),
    })
}

/// Writes `job`'s image to `out` (through a temporary file in the same
/// folder, read back and checked) and its sidecar next to it. Returns the
/// sidecar's path.
pub(crate) fn write(
    job: &CleanJob,
    input: &Path,
    out: &Path,
    registry: &Registry,
) -> Result<PathBuf> {
    let dir = out
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    out.file_name()
        .with_context(|| format!("{} has no file name", out.display()))?;
    // A new file with a random name, created exclusively, so it can't be an
    // existing file or a symlink to one (the input, say). It's removed if
    // anything below fails, and the output only appears once verified.
    let mut tmp = tempfile::Builder::new()
        .prefix(".gp-clean-")
        .suffix(".partial")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    std::io::Write::write_all(&mut tmp, &job.cleaned.bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .with_context(|| format!("writing {}", tmp.path().display()))?;
    let back =
        fs::read(tmp.path()).with_context(|| format!("reading back {}", tmp.path().display()))?;
    verify(&back, job, registry).context("the cleaned copy didn't verify")?;

    let sidecar = sidecar_path(out);
    let record = Sidecar {
        schema: 1,
        tool: "gp roms clean",
        input,
        input_bytes: job.image.file_bytes,
        input_sha256: &job.image.sha256_file,
        output: out,
        output_bytes: job.cleaned.bytes.len() as u64,
        sha256: &job.image.sha256,
        source: job.source.as_deref(),
        header_rule: job.cleaned.rule,
        header: hex::encode(job.cleaned.header),
        contradictions: job
            .cleaned
            .contradictions
            .iter()
            .map(ToString::to_string)
            .collect(),
        findings: job
            .cleaned
            .findings
            .iter()
            .map(dump::Finding::line)
            .collect(),
        removed: job.cleaned.removed(),
    };
    let text = format!(
        "# What `gp roms clean` removed to make {}. Header bytes are\n# quoted; game data never is.\n{}",
        out.display(),
        toml::to_string(&record).context("writing the sidecar")?
    );
    // The sidecar first, so a cleaned copy never appears without one.
    let mut side = tempfile::Builder::new()
        .prefix(".gp-clean-")
        .suffix(".toml.partial")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    std::io::Write::write_all(&mut side, text.as_bytes())
        .with_context(|| format!("writing {}", side.path().display()))?;
    side.persist(&sidecar)
        .with_context(|| format!("writing {}", sidecar.display()))?;
    tmp.persist(out)
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(sidecar)
}

/// The written copy has the input's identity, no junk, and the header gp-nes
/// would load it with.
fn verify(bytes: &[u8], job: &CleanJob, registry: &Registry) -> Result<()> {
    let (img, findings) = dump::inspect(bytes)?;
    if img.sha256 != job.image.sha256 {
        bail!(
            "its SHA-256 is {}, not the input's {}",
            img.sha256,
            job.image.sha256
        );
    }
    if !findings.is_empty() {
        bail!("it still has {}", findings[0].line());
    }
    let facts = registry
        .by_sha256(&img.sha256)
        .and_then(KnownSource::board_facts);
    if dump::runnable(bytes, facts.as_ref())? != bytes {
        bail!("its header isn't the one it would run with");
    }
    Ok(())
}

/// Refuses an output path that would put ROM data in a repo or replace the
/// input.
pub(crate) fn check_output(input: &Path, out: &Path, force: bool) -> Result<()> {
    for path in [out.to_path_buf(), sidecar_path(out)] {
        if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&path) {
            bail!(
                "{} is inside the git work tree at {}; ROMs never go in a repo, so write it outside",
                path.display(),
                repo.display()
            );
        }
        if path.exists() && !force {
            bail!(
                "{} already exists; pass --force to replace it",
                path.display()
            );
        }
    }
    let real = gp_evidence::paths::real_path;
    if real(input) == real(out) || real(input) == real(&sidecar_path(out)) {
        bail!("the output would replace the input; `gp roms clean` never changes its input");
    }
    Ok(())
}

pub(crate) fn clean(input: &Path, out: &Path, force: bool, registry: &Registry) -> Result<Outcome> {
    check_output(input, out, force)?;
    let bytes = fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let job = prepare(&bytes, registry).map_err(|e| match &e {
        DumpError::Source(s) => anyhow::anyhow!("{}: {}", input.display(), problem_line(s)),
        _ => anyhow::anyhow!("{}: {e}", input.display()),
    })?;
    let sidecar = write(&job, input, out, registry)?;
    let name = input
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (report, notes) = scan_findings(&bytes, &name, registry)?;
    let removed = job.cleaned.removed();
    let mut text = vec![format!(
        "{} -> {} ({})",
        input.display(),
        out.display(),
        job.source.as_deref().unwrap_or("unknown source")
    )];
    text.extend(notes.iter().map(|l| format!("  {l}")));
    let was = if job.cleaned.original_header == job.cleaned.header {
        String::new()
    } else {
        format!(", was {}", hex::encode(job.cleaned.original_header))
    };
    let from = match job.cleaned.rule {
        dump::HeaderRule::Registry => "the registry's board facts",
        dump::HeaderRule::File => "the file, by the header rules",
    };
    text.push(format!(
        "  header: {} from {from}{was}",
        hex::encode(job.cleaned.header)
    ));
    for c in &job.cleaned.contradictions {
        text.push(format!(
            "  header contradicted the registry, which wins: {c}"
        ));
    }
    if removed.is_empty() {
        text.push("  nothing to remove: the copy is identical to the input".into());
    }
    text.push(format!("  sidecar: {}", sidecar.display()));
    let known = job.source.is_some();
    if !known {
        text.push(
            "  the cleaned data matches no game's roms.toml, so it isn't verified (exit 1)".into(),
        );
    }
    Ok(Outcome {
        json: json!({
            "input": input,
            "output": out,
            "sidecar": sidecar,
            "sha256": job.image.sha256,
            "source": job.source,
            "header_rule": job.cleaned.rule,
            "header": hex::encode(job.cleaned.header),
            "removed": removed,
            "report": report,
        }),
        text: text.join("\n"),
        exit: u8::from(!known),
    })
}
