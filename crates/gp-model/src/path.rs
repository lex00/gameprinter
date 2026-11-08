//! Model paths: the names contract rules, evidence, diffs, queries and
//! selections use for parts of a model (docs/formats/model-path.md).
//!
//! ```text
//! path     := segment ("." segment)*
//! segment  := name ("[" index "]")?
//! name     := [A-Za-z_][A-Za-z0-9_]*
//! index    := "*" | number (".." number)? | name "=" value
//! value    := number | name | string
//! string   := '"' (any character but '"' and '\', or '\"' or '\\')* '"'
//! number   := "-"? (digits | "0x" hex | "$" hex)
//! ```
//!
//! The runtime selectors in `gp-runtime` (`ram[0x0770]`, `ram[0x0700..0x0710]`,
//! `cpu.pc`) are the subset without filters, and parse to the same segments
//! here; a test holds the two parsers to it. `[name=value]` filters are the
//! model's: `entities[role=enemy]` names every entity whose `role` is
//! `enemy`.

use std::fmt;

/// A parsed model path. With `[*]`, a range or a filter it's a pattern
/// that can name many parts of a model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Path {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Segment {
    pub name: String,
    pub index: Option<Index>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Index {
    /// One element of a list.
    At(i64),
    /// Elements from the first to before the second.
    Range(i64, i64),
    /// Every element of a list, or every entry of a table.
    All,
    /// The elements or entries whose field `name` equals `value`.
    Filter(String, Value),
}

/// A filter's value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    Int(i64),
    /// A bare `true` or `false`.
    Bool(bool),
    /// A bare name or a quoted string.
    Text(String),
}

/// One step of a path through an actual model: a table key or a list
/// position. A [`Concrete`] path names exactly one part.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    Key(String),
    Item(usize),
}

/// A path with no patterns in it, as found in a model.
pub type Concrete = Vec<Step>;

/// How deeply paths may nest, so a hostile one can't run long.
pub const MAX_SEGMENTS: usize = 64;

impl Path {
    pub fn parse(text: &str) -> Result<Self, String> {
        Parser { text, pos: 0 }.path()
    }

    /// True when the path names exactly one part: no `[*]`, range or
    /// filter.
    pub fn is_concrete(&self) -> bool {
        self.segments
            .iter()
            .all(|s| matches!(s.index, None | Some(Index::At(_))))
    }

    /// How specific the pattern is, for ordering rules: more steps first,
    /// then exact names and positions over filters and ranges over `[*]`.
    /// A name or position counts 3, a filter or range 2, `[*]` 1.
    pub fn specificity(&self) -> (usize, usize) {
        let mut steps = 0;
        let mut weight = 0;
        for s in &self.segments {
            steps += 1;
            weight += 3;
            if let Some(i) = &s.index {
                steps += 1;
                weight += match i {
                    Index::At(_) => 3,
                    Index::Range(..) | Index::Filter(..) => 2,
                    Index::All => 1,
                };
            }
        }
        (steps, weight)
    }

    /// Every part of `root` the path names, in the order the model holds
    /// them. Tables are walked in key order.
    pub fn resolve(&self, root: &toml::Value) -> Vec<Concrete> {
        let mut found: Vec<(Concrete, &toml::Value)> = vec![(Vec::new(), root)];
        for seg in &self.segments {
            let mut next = Vec::new();
            for (at, node) in found {
                let Some(child) = node.as_table().and_then(|t| t.get(&seg.name)) else {
                    continue;
                };
                let mut at = at;
                at.push(Step::Key(seg.name.clone()));
                match &seg.index {
                    None => next.push((at, child)),
                    Some(index) => {
                        for (step, node) in children(child, index) {
                            let mut at = at.clone();
                            at.push(step);
                            next.push((at, node));
                        }
                    }
                }
            }
            found = next;
        }
        found.into_iter().map(|(at, _)| at).collect()
    }
}

/// The children of `node` an index picks.
fn children<'a>(node: &'a toml::Value, index: &Index) -> Vec<(Step, &'a toml::Value)> {
    let all: Vec<(Step, &toml::Value)> = match node {
        toml::Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (Step::Item(i), v))
            .collect(),
        toml::Value::Table(t) => match index {
            Index::All | Index::Filter(..) => {
                t.iter().map(|(k, v)| (Step::Key(k.clone()), v)).collect()
            }
            // A table has no positions.
            Index::At(_) | Index::Range(..) => Vec::new(),
        },
        _ => Vec::new(),
    };
    all.into_iter()
        .filter(|(step, v)| match (index, step) {
            (Index::All, _) => true,
            (Index::At(n), Step::Item(i)) => *n >= 0 && *i as i64 == *n,
            (Index::Range(a, b), Step::Item(i)) => (*a..*b).contains(&(*i as i64)),
            (Index::Filter(field, want), _) => v
                .as_table()
                .and_then(|t| t.get(field))
                .is_some_and(|got| matches_value(got, want)),
            _ => false,
        })
        .collect()
}

fn matches_value(got: &toml::Value, want: &Value) -> bool {
    match (got, want) {
        (toml::Value::Integer(g), Value::Int(w)) => g == w,
        (toml::Value::String(g), Value::Text(w)) => g == w,
        (toml::Value::Boolean(g), Value::Bool(w)) => g == w,
        _ => false,
    }
}

/// Writes a concrete path in the grammar: `levels.main.spawn[0].entity`.
pub fn display(path: &[Step]) -> String {
    let mut out = String::new();
    for step in path {
        match step {
            Step::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Step::Item(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

/// True when `name` can be a path segment's name.
pub fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", seg.name)?;
            match &seg.index {
                None => {}
                Some(Index::At(n)) => write!(f, "[{n}]")?,
                Some(Index::Range(a, b)) => write!(f, "[{a}..{b}]")?,
                Some(Index::All) => write!(f, "[*]")?,
                Some(Index::Filter(k, v)) => write!(f, "[{k}={v}]")?,
            }
        }
        Ok(())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Text(s) if is_name(s) && s != "true" && s != "false" => write!(f, "{s}"),
            Value::Text(s) => {
                write!(f, "\"")?;
                for c in s.chars() {
                    if c == '"' || c == '\\' {
                        write!(f, "\\")?;
                    }
                    write!(f, "{c}")?;
                }
                write!(f, "\"")
            }
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn error(&self, what: &str) -> String {
        format!("{what} at column {} of `{}`", self.pos + 1, self.text)
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    fn path(&mut self) -> Result<Path, String> {
        let mut segments = Vec::new();
        loop {
            if segments.len() == MAX_SEGMENTS {
                return Err(self.error(&format!("more than {MAX_SEGMENTS} segments")));
            }
            let name = self.name()?;
            let index = if self.rest().starts_with('[') {
                self.pos += 1;
                let index = self.index()?;
                if !self.rest().starts_with(']') {
                    return Err(self.error("expected `]`"));
                }
                self.pos += 1;
                Some(index)
            } else {
                None
            };
            segments.push(Segment { name, index });
            if self.rest().starts_with('.') && !self.rest().starts_with("..") {
                self.pos += 1;
            } else if self.rest().is_empty() {
                return Ok(Path { segments });
            } else {
                return Err(self.error("expected `.`, `[` or the end of the path"));
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let rest = self.rest();
        let len = rest
            .char_indices()
            .find(|&(i, c)| !(c.is_ascii_alphabetic() || c == '_' || (i > 0 && c.is_ascii_digit())))
            .map_or(rest.len(), |(i, _)| i);
        if len == 0 {
            return Err(self.error("expected a name"));
        }
        self.pos += len;
        Ok(rest[..len].to_owned())
    }

    fn index(&mut self) -> Result<Index, String> {
        self.skip_space();
        let index = if self.rest().starts_with('*') {
            self.pos += 1;
            Index::All
        } else if self
            .rest()
            .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            let field = self.name()?;
            self.skip_space();
            if !self.rest().starts_with('=') {
                return Err(self.error("expected `=` after the filter's field name"));
            }
            self.pos += 1;
            self.skip_space();
            Index::Filter(field, self.value()?)
        } else {
            let first = self.number()?;
            self.skip_space();
            if self.rest().starts_with("..") {
                self.pos += 2;
                self.skip_space();
                let end = self.number()?;
                if end <= first {
                    return Err(self.error("a range must end after it starts"));
                }
                Index::Range(first, end)
            } else {
                Index::At(first)
            }
        };
        self.skip_space();
        Ok(index)
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.rest().chars().next() {
            Some('"') => {
                self.pos += 1;
                let mut out = String::new();
                let mut chars = self.rest().char_indices();
                while let Some((i, c)) = chars.next() {
                    match c {
                        '"' => {
                            self.pos += i + 1;
                            return Ok(Value::Text(out));
                        }
                        '\\' => match chars.next() {
                            Some((_, e @ ('"' | '\\'))) => out.push(e),
                            _ => {
                                self.pos += i;
                                return Err(self.error("only `\\\"` and `\\\\` escape in a string"));
                            }
                        },
                        c => out.push(c),
                    }
                }
                Err(self.error("the string isn't closed"))
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let name = self.name()?;
                Ok(match name.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => Value::Text(name),
                })
            }
            Some(c) if c.is_ascii_digit() || c == '-' || c == '$' => Ok(Value::Int(self.number()?)),
            _ => Err(self.error("expected a number, a name or a quoted string")),
        }
    }

    fn number(&mut self) -> Result<i64, String> {
        let rest = self.rest();
        let (neg, body) = match rest.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, rest),
        };
        let (radix, digits_at) = if body.starts_with("0x") || body.starts_with("0X") {
            (16, 2)
        } else if body.starts_with('$') {
            (16, 1)
        } else {
            (10, 0)
        };
        let digits = &body[digits_at..];
        // `_` separates digits, as the runtime selectors allow.
        let len = digits
            .find(|c: char| !c.is_digit(radix) && c != '_')
            .unwrap_or(digits.len());
        let text: String = digits[..len].chars().filter(|c| *c != '_').collect();
        if text.is_empty() {
            return Err(self.error("expected a number"));
        }
        let n =
            i64::from_str_radix(&text, radix).map_err(|_| self.error("the number is too large"))?;
        self.pos += usize::from(neg) + digits_at + len;
        Ok(if neg { -n } else { n })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Path {
        Path::parse(text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn parses_names_indexes_ranges_wildcards_and_filters() {
        let path = p("entities.player.jump.speed");
        assert_eq!(path.segments.len(), 4);
        assert!(path.is_concrete());
        assert_eq!(
            p("levels.main.spawn[1]").segments[2].index,
            Some(Index::At(1))
        );
        assert_eq!(
            p("ram[0x0700..0x0710]").segments[0].index,
            Some(Index::Range(0x700, 0x710))
        );
        assert_eq!(p("ram[$10]").segments[0].index, Some(Index::At(0x10)));
        assert_eq!(p("entities[*].role").segments[0].index, Some(Index::All));
        assert_eq!(
            p("entities[role=enemy]").segments[0].index,
            Some(Index::Filter("role".into(), Value::Text("enemy".into())))
        );
        assert_eq!(
            p("levels.main.spawn[entity = \"slug \\\"a\\\"\"]").segments[2].index,
            Some(Index::Filter(
                "entity".into(),
                Value::Text("slug \"a\"".into())
            ))
        );
        assert_eq!(
            p("x[n=-3]").segments[0].index,
            Some(Index::Filter("n".into(), Value::Int(-3)))
        );
        assert!(!p("entities[role=enemy].contact").is_concrete());
    }

    #[test]
    fn prints_what_it_parses() {
        for text in [
            "entities.player.jump.speed",
            "levels.main.spawn[0].entity",
            "ram[1792..1808]",
            "entities[*].role",
            "entities[role=enemy].contact.bounce",
            "a[b=\"two words\"]",
            "a[b=\"true\"]",
            "a[b=-4]",
            "a[b=true]",
        ] {
            assert_eq!(p(text).to_string(), text);
            assert_eq!(p(&p(text).to_string()), p(text));
        }
    }

    #[test]
    fn refuses_malformed_paths() {
        for text in [
            "",
            ".a",
            "a.",
            "a..b",
            "1a",
            "a[",
            "a[]",
            "a[1",
            "a[2..1]",
            "a[2..2]",
            "a[b=]",
            "a[b=\"x]",
            "a[b=\"\\n\"]",
            "a b",
            "a-b",
            "a[1][2]",
            "a[0x]",
            "a[0x_]",
            "a[99999999999999999999]",
        ] {
            assert!(Path::parse(text).is_err(), "{text}");
        }
        let deep = vec!["a"; MAX_SEGMENTS + 1].join(".");
        assert!(Path::parse(&deep).is_err());
    }

    #[test]
    fn specificity_orders_names_over_filters_over_wildcards() {
        let s = |t: &str| p(t).specificity();
        assert!(s("entities.slug") > s("entities[role=enemy]"));
        assert!(s("entities[role=enemy]") > s("entities[*]"));
        assert!(s("entities[*].contact") > s("entities.slug"));
        assert!(s("entities.slug.contact") > s("entities[role=enemy].contact"));
    }

    #[test]
    fn resolves_against_a_model_tree() {
        let tree: toml::Value = toml::from_str(
            r#"
            [entities.player]
            role = "player"
            [entities.slug]
            role = "enemy"
            flying = false
            [entities.bat]
            role = "enemy"
            [[levels.main.spawn]]
            entity = "slug"
            [[levels.main.spawn]]
            entity = "bat"
            [[levels.main.spawn]]
            entity = "slug"
            "#,
        )
        .unwrap();
        let names =
            |t: &str| -> Vec<String> { p(t).resolve(&tree).iter().map(|c| display(c)).collect() };
        assert_eq!(
            names("entities[role=enemy]"),
            ["entities.bat", "entities.slug"]
        );
        assert_eq!(names("entities[*].role").len(), 3);
        assert_eq!(names("entities[flying=false]"), ["entities.slug"]);
        assert_eq!(names("entities[flying=\"false\"]"), Vec::<String>::new());
        assert_eq!(names("entities.player.role"), ["entities.player.role"]);
        assert_eq!(names("entities.ghost"), Vec::<String>::new());
        assert_eq!(
            names("levels.main.spawn[1].entity"),
            ["levels.main.spawn[1].entity"]
        );
        assert_eq!(
            names("levels.main.spawn[entity=slug]"),
            ["levels.main.spawn[0]", "levels.main.spawn[2]"]
        );
        assert_eq!(names("levels.main.spawn[1..3]").len(), 2);
        assert_eq!(names("levels.main.spawn[-1]"), Vec::<String>::new());
        assert_eq!(names("entities[0]"), Vec::<String>::new());
        assert_eq!(
            p("levels.main.spawn[2]").to_string(),
            display(&p("levels.main.spawn[2]").resolve(&tree)[0])
        );
    }

    /// Every runtime selector form parses here to the same names and
    /// indexes, so the selectors stay a subset of the model path grammar.
    #[test]
    fn runtime_selectors_are_a_subset() {
        use gp_runtime::select::{Index as RIndex, Selector};
        for text in [
            "ram[0x0770]",
            "ram[$770]",
            "ram[1904]",
            "ram[0x0700..0x0710]",
            "ram[*]",
            "cpu.pc",
            "apu.noise.shift",
            "oam[-1]",
            "nametables[0..1024]",
            "frame",
            "_a1.b_2[3]",
            "ram[0x07_70]",
            "ram[1_000..2_000]",
        ] {
            let sel = Selector::parse(text).unwrap();
            let path = p(text);
            assert_eq!(sel.segments.len(), path.segments.len(), "{text}");
            for (s, m) in sel.segments.iter().zip(&path.segments) {
                assert_eq!(s.name, m.name, "{text}");
                let want = s.index.map(|i| match i {
                    RIndex::At(n) => Index::At(n),
                    RIndex::Range(a, b) => Index::Range(a, b),
                    RIndex::All => Index::All,
                });
                assert_eq!(want, m.index, "{text}");
            }
        }
        // And the filters the selectors leave to the model are refused there.
        assert!(Selector::parse("entities[role=enemy]").is_err());
    }
}
