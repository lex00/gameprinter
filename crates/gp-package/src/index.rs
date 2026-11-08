//! A game's themes index (docs/theme-publishing.md): `themes/index/<game>.toml`
//! in the repo, the source of truth, and the same bytes on each mirror at
//! `index/themes/<game>.toml`. It names packages by hash, never by URL.
//!
//! It is TOML of a fixed shape, so a runtime reads it with this code and
//! no TOML parser: `format` and `game`, then a `[[theme]]` table a theme,
//! each key a string or a whole number on a line of its own.
//!
//! ```toml
//! format = "0.1.0"
//! game = "rivet-rodeo"
//!
//! [[theme]]
//! id = "night"
//! title = "Night Shift"
//! package = "<64 hex digits>"
//! ```

/// The index format this code writes and reads.
pub const FORMAT: &str = "0.1.0";

/// The folder of the indexes in the repo, one file a game.
pub const REPO_DIR: &str = "themes/index";

/// A theme as the index lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Theme {
    /// The theme's id (theme.toml's), unique in the index.
    pub id: String,
    pub title: String,
    pub author: String,
    /// An SPDX expression.
    pub license: String,
    /// The package id: the hash of its manifest.
    pub package: String,
    /// The model it was drawn for (model.toml's `name`).
    pub model: String,
    /// The digest of the picture list the look was drawn against: a
    /// runtime swaps pictures by number, so it offers the theme only when
    /// its own build's digest is this one ([`pictures_digest`]).
    pub fits: String,
    /// The hash of the theme kit (kit.toml) the theme was made against.
    pub kit: String,
    /// What the package holds: `look` (a look drawn by gp, which `fits`
    /// keys) or `source` (the theme's own files, drawn by the player's
    /// runtime). Empty in an entry written before the key was added,
    /// which is a look.
    pub kind: String,
    /// The models the theme was checked on, by name, a space between.
    pub models: String,
    /// Each checked model's slot fingerprint ([`slots_digest`]), as
    /// `model:digest`, a space between, in the order of `models`. A
    /// runtime whose model and fingerprint are one of these has every
    /// art slot and sound role the theme was checked against.
    pub slots: String,
    /// Each model the theme was made for (theme.toml's `for`, else the
    /// model it was checked on) with its result, as `model:result`, a
    /// space between: `pass` when every check passed on it, `unchecked`
    /// when it wasn't checked (the package's checks.json says why). A
    /// runtime offers the theme on a model whose result is `pass` and
    /// lists it greyed out on an unchecked one. Empty in an entry written
    /// before the key was added.
    pub results: String,
    /// Each checked model's look digest, as `model:digest`, a space
    /// between: the BLAKE3 hash of the look drawn there over the lifted
    /// art (the file hashes and paths, a line each, sorted). Drawing is
    /// deterministic, so `gp theme recheck` with the same ROM and gp draws
    /// the same digest. Empty in an entry written before the key was
    /// added.
    pub looks: String,
    /// The hash of the preview picture, a file of the package. Empty for a
    /// source package, which holds no picture gp drew.
    pub preview: String,
    /// Who signed the manifest, as `allowed_signers` names them.
    pub signer: String,
    /// The checks' results, a line each: `pass ...`, or `not checked ...`.
    pub originality: String,
    pub readability: String,
    pub geometry: String,
    /// The package's size in bytes, its files together.
    pub size: u64,
}

/// A game's index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub format: String,
    pub game: String,
    pub themes: Vec<Theme>,
}

/// The fingerprint of a model's art slots and sound roles: the BLAKE3 hash
/// of the lines (one a slot or role, from `gp theme kit`), sorted, each
/// with a newline, after a first line naming the fingerprint's version.
/// The order the lines come in doesn't matter.
pub fn slots_digest(lines: &[impl AsRef<str>]) -> String {
    let mut v: Vec<&str> = lines.iter().map(AsRef::as_ref).collect();
    v.sort_unstable();
    let mut s = String::from("gp-slots 1\n");
    for l in v {
        s.push_str(l);
        s.push('\n');
    }
    crate::hash(s.as_bytes())
}

impl Theme {
    /// The slot fingerprint the theme records for `model`, if it was
    /// checked on it.
    pub fn slots_for(&self, model: &str) -> Option<&str> {
        self.slots
            .split_whitespace()
            .filter_map(|p| p.split_once(':'))
            .find(|(m, _)| *m == model)
            .map(|(_, d)| d)
    }

    /// Whether the theme was checked on `model` with the fingerprint
    /// `slots`: the model is in `models` and its recorded fingerprint is
    /// this one.
    pub fn fits_model(&self, model: &str, slots: &str) -> bool {
        self.models.split_whitespace().any(|m| m == model) && self.slots_for(model) == Some(slots)
    }

    /// Records a check on `model` with its fingerprint, in place of an
    /// earlier one on the same model.
    pub fn record_model(&mut self, model: &str, slots: &str) {
        let mut ms: Vec<&str> = self
            .models
            .split_whitespace()
            .filter(|m| *m != model)
            .collect();
        ms.push(model);
        let mut pairs: Vec<String> = self
            .slots
            .split_whitespace()
            .filter(|p| p.split_once(':').is_some_and(|(m, _)| m != model))
            .map(str::to_owned)
            .collect();
        pairs.push(format!("{model}:{slots}"));
        self.models = ms.join(" ");
        self.slots = pairs.join(" ");
    }

    /// The result recorded for `model`: `pass`, `unchecked`, or `None`
    /// when the theme wasn't made for it.
    pub fn result_for(&self, model: &str) -> Option<&str> {
        pair_of(&self.results, model)
    }

    /// Records `model`'s result, in place of an earlier one.
    pub fn record_result(&mut self, model: &str, result: &str) {
        put_pair(&mut self.results, model, result);
    }

    /// The look digest recorded for `model`, if it was checked on it.
    pub fn look_for(&self, model: &str) -> Option<&str> {
        pair_of(&self.looks, model)
    }

    /// Records `model`'s look digest, in place of an earlier one.
    pub fn record_look(&mut self, model: &str, digest: &str) {
        put_pair(&mut self.looks, model, digest);
    }
}

/// The value of `model` in a `model:value` list, a space between.
fn pair_of<'a>(list: &'a str, model: &str) -> Option<&'a str> {
    list.split_whitespace()
        .filter_map(|p| p.split_once(':'))
        .find(|(m, _)| *m == model)
        .map(|(_, v)| v)
}

/// Sets `model`'s value in a `model:value` list, in place of an earlier
/// one, at the end.
fn put_pair(list: &mut String, model: &str, value: &str) {
    let mut pairs: Vec<String> = list
        .split_whitespace()
        .filter(|p| p.split_once(':').is_some_and(|(m, _)| m != model))
        .map(str::to_owned)
        .collect();
    pairs.push(format!("{model}:{value}"));
    *list = pairs.join(" ");
}

/// The digest of a build's picture list: the BLAKE3 hash of the picture
/// file names, a line each, in the build's order.
pub fn pictures_digest(names: &[impl AsRef<str>]) -> String {
    let mut s = String::new();
    for n in names {
        s.push_str(n.as_ref());
        s.push('\n');
    }
    crate::hash(s.as_bytes())
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote(v: &str) -> Result<String, String> {
    let inner = v
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .ok_or_else(|| format!("`{v}` isn't a string"))?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let h: String = it.by_ref().take(4).collect();
                    let c = u32::from_str_radix(&h, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| format!("`\\u{h}` in `{v}`"))?;
                    out.push(c);
                }
                _ => return Err(format!("an escape in `{v}`")),
            },
            '"' => return Err(format!("a quote inside `{v}`")),
            c => out.push(c),
        }
    }
    Ok(out)
}

impl Index {
    pub fn new(game: &str) -> Self {
        Self {
            format: FORMAT.into(),
            game: game.into(),
            themes: Vec::new(),
        }
    }

    pub fn theme(&self, id: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.id == id)
    }

    /// Adds a theme, in place of one with its id (a new version).
    pub fn put(&mut self, t: Theme) {
        match self.themes.iter_mut().find(|x| x.id == t.id) {
            Some(x) => *x = t,
            None => self.themes.push(t),
        }
    }

    pub fn write(&self) -> String {
        let mut s = format!(
            "# The themes published for {} (docs/theme-publishing.md), written by\n# `gp theme publish`. Packages by hash; the mirrors are in mirrors.toml.\nformat = {}\ngame = {}\n",
            self.game,
            quote(&self.format),
            quote(&self.game)
        );
        for t in &self.themes {
            s.push_str("\n[[theme]]\n");
            for (k, v) in [
                ("id", &t.id),
                ("title", &t.title),
                ("author", &t.author),
                ("license", &t.license),
                ("package", &t.package),
                ("model", &t.model),
                ("fits", &t.fits),
                ("kit", &t.kit),
                ("kind", &t.kind),
                ("models", &t.models),
                ("slots", &t.slots),
                ("results", &t.results),
                ("looks", &t.looks),
                ("preview", &t.preview),
                ("signer", &t.signer),
                ("originality", &t.originality),
                ("readability", &t.readability),
                ("geometry", &t.geometry),
            ] {
                s.push_str(&format!("{k} = {}\n", quote(v)));
            }
            s.push_str(&format!("size = {}\n", t.size));
        }
        s
    }

    /// Reads an index in the shape [`Index::write`] writes. Unknown keys
    /// are kept out, so a newer index with more keys still reads.
    pub fn read(text: &str) -> Result<Self, String> {
        let mut ix = Index::default();
        let mut cur: Option<Theme> = None;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            let at = |e: String| format!("line {}: {e}", n + 1);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[theme]]" {
                if let Some(t) = cur.take() {
                    ix.themes.push(t);
                }
                cur = Some(Theme::default());
                continue;
            }
            if line.starts_with('[') {
                return Err(at(format!("`{line}` isn't a table this index has")));
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| at(format!("`{line}` isn't `key = value`")))?;
            let (k, v) = (k.trim(), v.trim());
            match cur.as_mut() {
                None => match k {
                    "format" => ix.format = unquote(v).map_err(at)?,
                    "game" => ix.game = unquote(v).map_err(at)?,
                    _ => {}
                },
                Some(t) => {
                    if k == "size" {
                        t.size = v.parse().map_err(|_| at(format!("`{v}` isn't a size")))?;
                        continue;
                    }
                    let s = unquote(v).map_err(at)?;
                    let slot = match k {
                        "id" => &mut t.id,
                        "title" => &mut t.title,
                        "author" => &mut t.author,
                        "license" => &mut t.license,
                        "package" => &mut t.package,
                        "model" => &mut t.model,
                        "fits" => &mut t.fits,
                        "kit" => &mut t.kit,
                        "kind" => &mut t.kind,
                        "models" => &mut t.models,
                        "slots" => &mut t.slots,
                        "results" => &mut t.results,
                        "looks" => &mut t.looks,
                        "preview" => &mut t.preview,
                        "signer" => &mut t.signer,
                        "originality" => &mut t.originality,
                        "readability" => &mut t.readability,
                        "geometry" => &mut t.geometry,
                        _ => continue,
                    };
                    *slot = s;
                }
            }
        }
        if let Some(t) = cur.take() {
            ix.themes.push(t);
        }
        if ix.format.split('.').next() != FORMAT.split('.').next() || ix.format.is_empty() {
            return Err(format!("index format `{}`; this reads {FORMAT}", ix.format));
        }
        for t in &ix.themes {
            if t.id.is_empty() || !crate::is_hash(&t.package) {
                return Err(format!(
                    "theme `{}`: an id and a package hash are needed",
                    t.id
                ));
            }
        }
        let mut ids: Vec<&str> = ix.themes.iter().map(|t| t.id.as_str()).collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("theme `{}` is listed twice", w[0]));
        }
        Ok(ix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let mut ix = Index::new("rivet-rodeo");
        ix.put(Theme {
            id: "night".into(),
            title: "Night \"Shift\"\tTwo".into(),
            author: "A. Maker \\ friends".into(),
            license: "CC0-1.0".into(),
            package: crate::hash(b"m"),
            model: "rivet_rodeo".into(),
            fits: pictures_digest(&["frame-0-0.png", "frame-1-0.png"]),
            kit: crate::hash(b"kit"),
            kind: "look".into(),
            models: "rivet_rodeo".into(),
            slots: format!(
                "rivet_rodeo:{}",
                slots_digest(&["art font font 8x8 frames=10 colors=any"])
            ),
            results: "rivet_rodeo:pass".into(),
            looks: format!("rivet_rodeo:{}", crate::hash(b"look")),
            preview: crate::hash(b"p"),
            signer: "lex00".into(),
            originality: "pass: art at most 0.540".into(),
            readability: "pass".into(),
            geometry: "not checked: no geometry check in this gp".into(),
            size: 12345,
        });
        ix
    }

    #[test]
    fn an_index_round_trips_and_toml_reads_it_the_same() {
        let ix = sample();
        let text = ix.write();
        assert_eq!(Index::read(&text).unwrap(), ix);
        // The same values through a TOML parser: the shape is TOML.
        let t: toml::Table = text.parse().unwrap();
        assert_eq!(t["game"].as_str(), Some("rivet-rodeo"));
        let th = &t["theme"].as_array().unwrap()[0];
        assert_eq!(th["title"].as_str(), Some(ix.themes[0].title.as_str()));
        assert_eq!(th["author"].as_str(), Some(ix.themes[0].author.as_str()));
        assert_eq!(th["size"].as_integer(), Some(12345));
        // A new version replaces the old by id.
        let mut ix2 = ix.clone();
        let mut t2 = ix.themes[0].clone();
        t2.package = crate::hash(b"m2");
        ix2.put(t2);
        assert_eq!(ix2.themes.len(), 1);
        assert_eq!(ix2.themes[0].package, crate::hash(b"m2"));
    }

    #[test]
    fn a_bad_index_is_refused_and_unknown_keys_are_passed_over() {
        let text = sample().write();
        assert!(Index::read(&text.replace("format = \"0.1.0\"", "format = \"9.0.0\"")).is_err());
        assert!(
            Index::read(&format!(
                "{text}\n[[theme]]\nid = \"x\"\npackage = \"abc\"\n"
            ))
            .is_err()
        );
        let twice = format!("{text}{}", &text[text.find("[[theme]]").unwrap()..]);
        assert!(Index::read(&twice).unwrap_err().contains("twice"));
        assert!(Index::read(&format!("{text}[other]\n")).is_err());
        assert!(Index::read(&text.replace("size = 12345", "size = big")).is_err());
        let more = text.replace(
            "size = 12345",
            "size = 12345\nlater = \"a key from a newer gp\"",
        );
        assert_eq!(Index::read(&more).unwrap(), sample());
    }

    #[test]
    fn an_index_from_before_the_slot_keys_reads_with_them_empty() {
        let text = sample().write();
        let old: String = text
            .lines()
            .filter(|l| {
                !["kind", "models", "slots", "results", "looks"]
                    .iter()
                    .any(|k| l.starts_with(&format!("{k} =")))
            })
            .map(|l| format!("{l}\n"))
            .collect();
        let t = &Index::read(&old).unwrap().themes[0];
        assert_eq!(
            (t.kind.as_str(), t.models.as_str(), t.slots.as_str()),
            ("", "", "")
        );
        assert_eq!(t.result_for("rivet_rodeo"), None);
        assert_eq!(t.look_for("rivet_rodeo"), None);
        assert!(!t.fits_model("rivet_rodeo", &slots_digest(&["x"])));
    }

    /// A made-up fingerprint's hash, written out: the same on every
    /// machine and run, since it hashes text only.
    #[test]
    fn the_slot_fingerprint_is_fixed_and_order_free() {
        let lines = [
            "sound jump sfx loops=false",
            "art entity.a sprite 16x16 frames=2 colors=any",
        ];
        let d = slots_digest(&lines);
        assert_eq!(d, slots_digest(&[lines[1], lines[0]]));
        assert_eq!(
            d,
            crate::hash(b"gp-slots 1\nart entity.a sprite 16x16 frames=2 colors=any\nsound jump sfx loops=false\n")
        );
        assert_ne!(d, slots_digest(&lines[..1]));
    }

    #[test]
    fn a_theme_fits_each_model_whose_fingerprint_it_records() {
        let (a, b) = (slots_digest(&["art a"]), slots_digest(&["art a", "art b"]));
        let mut t = Theme::default();
        t.record_model("small", &a);
        t.record_model("big", &b);
        assert_eq!(t.models, "small big");
        assert!(t.fits_model("small", &a));
        assert!(t.fits_model("big", &b));
        assert!(
            !t.fits_model("small", &b),
            "another fingerprint: the model changed"
        );
        assert!(!t.fits_model("other", &a), "a model it wasn't checked on");
        // A recheck replaces the model's fingerprint.
        t.record_model("small", &b);
        assert_eq!(t.models, "big small");
        assert!(t.fits_model("small", &b) && !t.fits_model("small", &a));
        // Through the index's text and back.
        let mut ix = Index::new("demo");
        t.id = "night".into();
        t.package = crate::hash(b"m");
        ix.put(t.clone());
        let back = Index::read(&ix.write()).unwrap();
        assert_eq!(back.themes[0], t);
        assert!(back.themes[0].fits_model("big", &b));
    }

    #[test]
    fn each_model_a_theme_was_made_for_has_its_result() {
        let mut t = Theme::default();
        t.record_result("small", "pass");
        t.record_result("big", "unchecked");
        assert_eq!(t.result_for("small"), Some("pass"));
        assert_eq!(t.result_for("big"), Some("unchecked"));
        assert_eq!(t.result_for("other"), None);
        // A recheck replaces the model's result.
        t.record_result("big", "pass");
        assert_eq!(t.results, "small:pass big:pass");
        // And records the look it drew there beside the publisher's.
        let (a, b) = (crate::hash(b"a"), crate::hash(b"b"));
        t.record_look("small", &a);
        t.record_look("big", &b);
        assert_eq!(t.look_for("big"), Some(b.as_str()));
        assert_eq!(t.look_for("other"), None);
        t.record_look("small", &b);
        assert_eq!(t.looks, format!("big:{b} small:{b}"));
    }
}
