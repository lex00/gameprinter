//! A TOML file's fields with where they are: each `key = value`, with its
//! full dotted path and the bytes its value spans, so a match found in the
//! text can be named by the field it is in.
//!
//! A table of scalars inline or an array of scalars is one field; an inline
//! table's or an array of tables' own keys are fields of their own. The
//! lexer reads what TOML allows and is lenient about the rest: a file that
//! doesn't parse still gives the fields it could read.

/// What a field's value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Int,
    Str,
    Array,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub path: String,
    /// The last key of the path.
    pub key: String,
    pub value: Value,
    /// Byte offsets of the value in the text.
    pub start: usize,
    pub end: usize,
    /// The 1-based line the key is on.
    pub line: usize,
}

/// A group of consecutive fields or array items, for the reordered and
/// scaled detectors: (start, end) byte spans of each array and each table
/// or inline table.
pub type Span = (usize, usize);

pub struct Lexed {
    pub fields: Vec<Field>,
    /// Arrays, inline tables and tables (between headers), by byte span.
    pub groups: Vec<Span>,
}

struct Lexer<'a> {
    t: &'a [u8],
    s: &'a str,
    i: usize,
    fields: Vec<Field>,
    groups: Vec<Span>,
    /// Counts of arrays of tables seen, by path, for `a[n]` paths.
    aot: std::collections::HashMap<String, usize>,
    /// Where each line starts.
    starts: Vec<usize>,
}

pub fn lex(text: &str) -> Lexed {
    let mut l = Lexer {
        t: text.as_bytes(),
        s: text,
        i: 0,
        fields: Vec::new(),
        groups: Vec::new(),
        aot: Default::default(),
        starts: std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
    };
    let mut header = String::new();
    let mut section_start = 0;
    while l.i < l.t.len() {
        l.skip_ws_nl();
        if l.i >= l.t.len() {
            break;
        }
        match l.t[l.i] {
            b'#' => l.skip_comment(),
            b'[' => {
                if section_start < l.i {
                    l.groups.push((section_start, l.i));
                }
                let double = l.t.get(l.i + 1) == Some(&b'[');
                l.i += if double { 2 } else { 1 };
                let keys = l.keys(b']');
                l.i += if double { 2 } else { 1 };
                let p = keys.join(".");
                header = if double {
                    let n = l.aot.entry(p.clone()).or_insert(0);
                    let h = format!("{p}[{n}]");
                    *n += 1;
                    h
                } else {
                    p
                };
                l.skip_line();
                section_start = l.i;
            }
            _ => {
                let start = l.i;
                let keys = l.keys(b'=');
                if l.t.get(l.i) != Some(&b'=') {
                    // Not a key line: skip it.
                    l.i = start;
                    l.skip_line();
                    continue;
                }
                l.i += 1;
                let line = l.line_of(start);
                let path = join(&header, &keys.join("."));
                let key = keys.last().cloned().unwrap_or_default();
                l.value(&path, &key, line);
            }
        }
    }
    if section_start < l.t.len() {
        l.groups.push((section_start, l.t.len()));
    }
    Lexed {
        fields: l.fields,
        groups: l.groups,
    }
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_owned(),
        (_, true) => a.to_owned(),
        _ => format!("{a}.{b}"),
    }
}

impl Lexer<'_> {
    fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|&s| s <= at)
    }

    fn skip_ws(&mut self) {
        while self.i < self.t.len() && matches!(self.t[self.i], b' ' | b'\t' | b'\r') {
            self.i += 1;
        }
    }

    fn skip_ws_nl(&mut self) {
        while self.i < self.t.len() && self.t[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    /// Whitespace, newlines and comments, as inside an array.
    fn skip_space(&mut self) {
        loop {
            self.skip_ws_nl();
            if self.t.get(self.i) == Some(&b'#') {
                self.skip_comment();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        while self.i < self.t.len() && self.t[self.i] != b'\n' {
            self.i += 1;
        }
    }

    fn skip_line(&mut self) {
        self.skip_comment();
    }

    /// Dotted keys up to `stop` (not consumed).
    fn keys(&mut self, stop: u8) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            let Some(&c) = self.t.get(self.i) else {
                break;
            };
            if c == stop || c == b'\n' {
                break;
            }
            if c == b'"' || c == b'\'' {
                let start = self.i;
                self.string();
                let raw = &self.s[start..self.i];
                out.push(raw.trim_matches(|c| c == '"' || c == '\'').to_owned());
            } else if c == b'.' {
                self.i += 1;
            } else {
                let start = self.i;
                while self.i < self.t.len()
                    && (self.t[self.i].is_ascii_alphanumeric()
                        || matches!(self.t[self.i], b'_' | b'-'))
                {
                    self.i += 1;
                }
                if self.i == start {
                    // Not a key character: stop here.
                    break;
                }
                out.push(self.s[start..self.i].to_owned());
            }
        }
        out
    }

    /// A string of any of TOML's four kinds, from its opening quote.
    fn string(&mut self) {
        let q = self.t[self.i];
        let triple = self.t.get(self.i + 1) == Some(&q) && self.t.get(self.i + 2) == Some(&q);
        if triple {
            self.i += 3;
            while self.i < self.t.len() {
                if q == b'"' && self.t[self.i] == b'\\' {
                    self.i += 2;
                    continue;
                }
                if self.t[self.i] == q
                    && self.t.get(self.i + 1) == Some(&q)
                    && self.t.get(self.i + 2) == Some(&q)
                {
                    self.i += 3;
                    while self.t.get(self.i) == Some(&q) {
                        self.i += 1;
                    }
                    return;
                }
                self.i += 1;
            }
            return;
        }
        self.i += 1;
        while self.i < self.t.len() && self.t[self.i] != b'\n' {
            if q == b'"' && self.t[self.i] == b'\\' {
                self.i += 2;
                continue;
            }
            if self.t[self.i] == q {
                self.i += 1;
                return;
            }
            self.i += 1;
        }
    }

    /// A value, emitting its fields under `path`.
    fn value(&mut self, path: &str, key: &str, line: usize) {
        self.skip_ws();
        let start = self.i;
        let Some(&c) = self.t.get(self.i) else {
            return;
        };
        match c {
            b'{' => {
                self.i += 1;
                loop {
                    self.skip_space();
                    match self.t.get(self.i) {
                        None => break,
                        Some(b'}') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                        }
                        _ => {
                            let ks = self.i;
                            let keys = self.keys(b'=');
                            if self.t.get(self.i) != Some(&b'=') {
                                self.i = ks + 1;
                                continue;
                            }
                            self.i += 1;
                            let l = self.line_of(ks);
                            let p = join(path, &keys.join("."));
                            let k = keys.last().cloned().unwrap_or_default();
                            self.value(&p, &k, l);
                        }
                    }
                }
                self.groups.push((start, self.i));
            }
            b'[' => {
                self.i += 1;
                let mut tables = 0;
                loop {
                    self.skip_space();
                    match self.t.get(self.i) {
                        None => break,
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => self.i += 1,
                        Some(b'{') => {
                            let p = format!("{path}[{tables}]");
                            tables += 1;
                            let l = self.line_of(self.i);
                            self.value(&p, key, l);
                        }
                        Some(b'[') => {
                            // A nested array: part of this field.
                            self.nested_array();
                        }
                        Some(b'"' | b'\'') => self.string(),
                        _ => self.scalar(),
                    }
                }
                self.groups.push((start, self.i));
                if tables == 0 {
                    self.push(path, key, Value::Array, start, line);
                }
            }
            b'"' | b'\'' => {
                self.string();
                self.push(path, key, Value::Str, start, line);
            }
            _ => {
                self.scalar();
                let raw = self.s[start..self.i].trim();
                let int = raw.trim_start_matches(['+', '-']).replace('_', "");
                let is_int = !int.is_empty()
                    && (int.bytes().all(|c| c.is_ascii_digit())
                        || int
                            .strip_prefix("0x")
                            .is_some_and(|h| h.bytes().all(|c| c.is_ascii_hexdigit()))
                        || int
                            .strip_prefix("0b")
                            .is_some_and(|h| h.bytes().all(|c| c == b'0' || c == b'1'))
                        || int
                            .strip_prefix("0o")
                            .is_some_and(|h| h.bytes().all(|c| (b'0'..=b'7').contains(&c))));
                let v = if is_int { Value::Int } else { Value::Other };
                self.push(path, key, v, start, line);
            }
        }
    }

    fn nested_array(&mut self) {
        let start = self.i;
        let mut depth = 0;
        while self.i < self.t.len() {
            match self.t[self.i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        break;
                    }
                }
                b'"' | b'\'' => {
                    self.string();
                    continue;
                }
                b'#' => {
                    self.skip_comment();
                    continue;
                }
                _ => {}
            }
            self.i += 1;
        }
        self.groups.push((start, self.i));
    }

    fn scalar(&mut self) {
        while self.i < self.t.len() && !matches!(self.t[self.i], b',' | b']' | b'}' | b'\n' | b'#')
        {
            self.i += 1;
        }
    }

    fn push(&mut self, path: &str, key: &str, value: Value, start: usize, line: usize) {
        let mut end = self.i;
        while end > start && self.t[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        self.fields.push(Field {
            path: path.to_owned(),
            key: key.to_owned(),
            value,
            start,
            end,
            line,
        });
    }
}
