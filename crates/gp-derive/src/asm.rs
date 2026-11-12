//! A reader for 6502 assembly source: the lines of the dialects the pinned
//! disassemblies and Moss Hop are written in (ca65, asm6f, x816), parsed far
//! enough to read a fact's value from the line its evidence cites. A line
//! is a label, a constant definition, a data row or an instruction; the
//! numbers in them are expressions over constants, read without an
//! assembler. Nothing here needs the ROM.

use std::collections::BTreeMap;

/// How an instruction names its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Implied,
    Accumulator,
    Immediate,
    Address,
    AddressX,
    AddressY,
    Indirect,
    IndirectX,
    IndirectY,
}

/// An expression in an operand, a constant or a data row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(i64),
    Sym(String),
    /// `<x`, the low byte.
    Lo(Box<Expr>),
    /// `>x`, the high byte.
    Hi(Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
}

/// What a line says, after its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// Nothing, or only a comment.
    Empty,
    /// `NAME = expr` (also `:=`, `.equ`, `equ`).
    Const { name: String, value: Expr },
    /// A data row: bytes (`.db`, `.byte`) or words (`.dw`, `.word`), with
    /// `None` for an item that isn't a number (a string).
    Data { width: u8, items: Vec<Option<Expr>> },
    /// An instruction.
    Op {
        mnemonic: String,
        mode: Mode,
        arg: Option<Expr>,
    },
    /// A directive or anything else this reader doesn't need.
    Other,
}

/// One parsed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The label that starts it, without its colon. A local label keeps its
    /// sigil (`@loop`); ca65's anonymous label is `:`.
    pub label: Option<String>,
    pub stmt: Stmt,
    /// The text after `;`, if any.
    pub comment: Option<String>,
}

impl Line {
    /// A label that isn't local or anonymous.
    pub fn global_label(&self) -> Option<&str> {
        self.label.as_deref().filter(|l| is_global(l))
    }
}

/// Whether a label name is global: not `@local`, `.local`, `:` or `+`/`-`.
pub fn is_global(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

const MNEMONICS: &[&str] = &[
    "adc", "and", "asl", "bcc", "bcs", "beq", "bit", "bmi", "bne", "bpl", "brk", "bvc", "bvs",
    "clc", "cld", "cli", "clv", "cmp", "cpx", "cpy", "dec", "dex", "dey", "eor", "inc", "inx",
    "iny", "jmp", "jsr", "lda", "ldx", "ldy", "lsr", "nop", "ora", "pha", "php", "pla", "plp",
    "rol", "ror", "rti", "rts", "sbc", "sec", "sed", "sei", "sta", "stx", "sty", "tax", "tay",
    "tsx", "txa", "txs", "tya",
];

/// Whether `m` (lower case) is a 6502 mnemonic.
pub fn is_mnemonic(m: &str) -> bool {
    MNEMONICS.contains(&m)
}

/// The text before a `;` comment, leaving quoted `;` alone.
fn strip_comment(text: &str) -> &str {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (None, ';') => return &text[..i],
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
    }
    text
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '.'
}

/// Parses one line of source.
pub fn parse_line(text: &str) -> Line {
    let code = strip_comment(text);
    let trimmed = code.trim_start();
    let mut label = None;
    let mut rest = trimmed;
    // A label: an identifier (or ca65's bare `:`) followed by a colon.
    if let Some(r) = trimmed.strip_prefix(':')
        && !r.starts_with(':')
        && !r.starts_with('=')
    {
        label = Some(":".to_owned());
        rest = r;
    } else {
        let end = trimmed
            .char_indices()
            .find(|&(_, c)| !is_ident_char(c))
            .map_or(trimmed.len(), |(i, _)| i);
        if end > 0 && trimmed[end..].starts_with(':') && !trimmed[end..].starts_with(":=") {
            label = Some(trimmed[..end].to_owned());
            rest = &trimmed[end + 1..];
        }
    }
    let stmt = parse_stmt(rest.trim());
    let comment = text
        .get(code.len()..)
        .and_then(|c| c.strip_prefix(';'))
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    Line {
        label,
        stmt,
        comment,
    }
}

fn parse_stmt(s: &str) -> Stmt {
    if s.is_empty() {
        return Stmt::Empty;
    }
    let (head, tail) = match s.find(|c: char| c.is_whitespace()) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    };
    let lower = head.to_ascii_lowercase();
    // Constants.
    for sep in [":=", "="] {
        if let Some(i) = s.find(sep) {
            let name = s[..i].trim();
            if !name.is_empty()
                && name.chars().all(is_ident_char)
                && !is_mnemonic(&name.to_ascii_lowercase())
            {
                return match parse_expr(&s[i + sep.len()..]) {
                    Some(value) => Stmt::Const {
                        name: name.to_owned(),
                        value,
                    },
                    None => Stmt::Other,
                };
            }
        }
    }
    if let Some((op, value)) = tail.split_once(char::is_whitespace)
        && matches!(op.to_ascii_lowercase().as_str(), ".equ" | "equ" | ".set")
        && head.chars().all(is_ident_char)
    {
        return match parse_expr(value) {
            Some(value) => Stmt::Const {
                name: head.to_owned(),
                value,
            },
            None => Stmt::Other,
        };
    }
    let width = match lower.as_str() {
        ".db" | ".byte" | ".byt" | "db" | ".dc.b" => Some(1),
        ".dw" | ".word" | ".addr" | "dw" | ".dc.w" => Some(2),
        _ => None,
    };
    if let Some(width) = width {
        let items = split_items(tail)
            .into_iter()
            .map(|t| {
                if t.starts_with('"') {
                    None
                } else {
                    parse_expr(t)
                }
            })
            .collect();
        return Stmt::Data { width, items };
    }
    if is_mnemonic(&lower) {
        let (mode, arg) = parse_operand(tail);
        return Stmt::Op {
            mnemonic: lower,
            mode,
            arg,
        };
    }
    Stmt::Other
}

/// Splits a data row on commas outside quotes and parentheses.
fn split_items(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

fn parse_operand(t: &str) -> (Mode, Option<Expr>) {
    let t = t.trim();
    if t.is_empty() {
        return (Mode::Implied, None);
    }
    if t.eq_ignore_ascii_case("a") {
        return (Mode::Accumulator, None);
    }
    if let Some(v) = t.strip_prefix('#') {
        return (Mode::Immediate, parse_expr(v));
    }
    let lower = t.to_ascii_lowercase().replace(' ', "");
    if lower.starts_with('(') {
        if let Some(inner) = lower.strip_suffix("),y") {
            return (Mode::IndirectY, parse_expr(&t[1..inner.len()]));
        }
        if let Some(inner) = lower.strip_suffix(",x)") {
            return (Mode::IndirectX, parse_expr(&t[1..inner.len()]));
        }
        if lower.ends_with(')') && matching_paren(&lower) == Some(lower.len() - 1) {
            return (Mode::Indirect, parse_expr(&t[1..t.len() - 1]));
        }
    }
    if let Some(i) = lower.rfind(',') {
        let idx = &lower[i + 1..];
        let base = &t[..t.rfind(',').unwrap_or(t.len())];
        let mode = match idx {
            "x" => Mode::AddressX,
            "y" => Mode::AddressY,
            _ => return (Mode::Address, None),
        };
        return (mode, parse_expr(base));
    }
    (Mode::Address, parse_expr(t))
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parses an expression: numbers (`$1f`, `0x1f`, `%101`, `31`, `'a'`),
/// names, `<` and `>` bytes, unary `-` and `~`, and `+ - * / & | ^ << >>`
/// with C's precedence. `None` when it isn't one.
pub fn parse_expr(s: &str) -> Option<Expr> {
    let toks = lex(s.trim())?;
    let mut p = Parser { toks, at: 0 };
    let e = p.expr(0)?;
    (p.at == p.toks.len()).then_some(e)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(i64),
    Name(String),
    Op(&'static str),
    Open,
    Close,
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if let Some(r) = rest.strip_prefix('$') {
            let n = r.bytes().take_while(u8::is_ascii_hexdigit).count();
            if n == 0 {
                return None;
            }
            out.push(Tok::Num(i64::from_str_radix(&r[..n], 16).ok()?));
            i += 1 + n;
        } else if rest.starts_with("0x") || rest.starts_with("0X") {
            let r = &rest[2..];
            let n = r.bytes().take_while(u8::is_ascii_hexdigit).count();
            out.push(Tok::Num(i64::from_str_radix(&r[..n], 16).ok()?));
            i += 2 + n;
        } else if let Some(r) = rest.strip_prefix('%')
            && r.starts_with(['0', '1'])
        {
            let n = r.bytes().take_while(|c| *c == b'0' || *c == b'1').count();
            out.push(Tok::Num(i64::from_str_radix(&r[..n], 2).ok()?));
            i += 1 + n;
        } else if c.is_ascii_digit() {
            let n = rest.bytes().take_while(u8::is_ascii_digit).count();
            // asm6f and others also write hex as `1Fh`.
            let h = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
            if rest[h..].starts_with(['h', 'H']) {
                out.push(Tok::Num(i64::from_str_radix(&rest[..h], 16).ok()?));
                i += h + 1;
            } else {
                out.push(Tok::Num(rest[..n].parse().ok()?));
                i += n;
            }
        } else if c == '\'' && b.get(i + 2) == Some(&b'\'') {
            out.push(Tok::Num(i64::from(b[i + 1])));
            i += 3;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '.' {
            let n = rest
                .char_indices()
                .find(|&(_, c)| !is_ident_char(c))
                .map_or(rest.len(), |(k, _)| k);
            out.push(Tok::Name(rest[..n].to_owned()));
            i += n;
        } else if c == '(' || c == '[' {
            out.push(Tok::Open);
            i += 1;
        } else if c == ')' || c == ']' {
            out.push(Tok::Close);
            i += 1;
        } else {
            let op = ["<<", ">>", "+", "-", "*", "/", "&", "|", "^", "<", ">", "~"]
                .into_iter()
                .find(|o| rest.starts_with(o))?;
            out.push(Tok::Op(op));
            i += op.len();
        }
    }
    Some(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

fn prec(op: &str) -> Option<u8> {
    Some(match op {
        "|" => 1,
        "^" => 2,
        "&" => 3,
        "<<" | ">>" => 4,
        "+" | "-" => 5,
        "*" | "/" => 6,
        _ => return None,
    })
}

impl Parser {
    fn expr(&mut self, min: u8) -> Option<Expr> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Op(op)) = self.toks.get(self.at).cloned() {
            let Some(p) = prec(op) else { break };
            if p < min {
                break;
            }
            self.at += 1;
            let rhs = self.expr(p + 1)?;
            let c = match op {
                "<<" => 'l',
                ">>" => 'r',
                o => o.chars().next().expect("an operator"),
            };
            lhs = Expr::Bin(c, Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    fn unary(&mut self) -> Option<Expr> {
        let t = self.toks.get(self.at).cloned()?;
        self.at += 1;
        Some(match t {
            Tok::Num(n) => Expr::Num(n),
            Tok::Name(n) => Expr::Sym(n),
            Tok::Op("<") => Expr::Lo(Box::new(self.unary()?)),
            Tok::Op(">") => Expr::Hi(Box::new(self.unary()?)),
            Tok::Op("-") => Expr::Neg(Box::new(self.unary()?)),
            Tok::Op("~") => Expr::Not(Box::new(self.unary()?)),
            Tok::Open => {
                let e = self.expr(0)?;
                if self.toks.get(self.at) != Some(&Tok::Close) {
                    return None;
                }
                self.at += 1;
                e
            }
            _ => return None,
        })
    }
}

/// One source file, parsed.
#[derive(Debug, Clone)]
pub struct File {
    pub name: String,
    pub lines: Vec<Line>,
}

impl File {
    pub fn parse(name: &str, text: &str) -> Self {
        File {
            name: name.to_owned(),
            lines: text.lines().map(parse_line).collect(),
        }
    }

    /// Line `n`, counting from 1.
    pub fn line(&self, n: usize) -> Option<&Line> {
        n.checked_sub(1).and_then(|i| self.lines.get(i))
    }
}

/// The files of one disassembly, with the constants and labels they
/// define, so a symbol in one file resolves from another.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub files: BTreeMap<String, File>,
    /// Constant definitions: name -> (file, line, expression).
    pub consts: BTreeMap<String, (String, usize, Expr)>,
    /// Global labels: name -> (file, line).
    pub labels: BTreeMap<String, (String, usize)>,
    /// Label addresses, where a labels file gives them (`gp labels`).
    pub addresses: BTreeMap<String, i64>,
    /// How many operands and data items name each symbol, in every file.
    pub mentions: BTreeMap<String, usize>,
}

impl Source {
    pub fn add(&mut self, name: &str, text: &str) {
        let file = File::parse(name, text);
        for (i, l) in file.lines.iter().enumerate() {
            if let Stmt::Const { name: n, value } = &l.stmt {
                self.consts
                    .entry(n.clone())
                    .or_insert((name.to_owned(), i + 1, value.clone()));
            }
            if let Some(g) = l.global_label() {
                self.labels
                    .entry(g.to_owned())
                    .or_insert((name.to_owned(), i + 1));
            }
            let mut syms = Vec::new();
            match &l.stmt {
                Stmt::Op { arg: Some(e), .. } => symbols(e, &mut syms),
                Stmt::Data { items, .. } => {
                    items.iter().flatten().for_each(|e| symbols(e, &mut syms))
                }
                _ => {}
            }
            syms.sort();
            syms.dedup();
            for s in syms {
                *self.mentions.entry(s).or_default() += 1;
            }
        }
        self.files.insert(name.to_owned(), file);
    }

    /// Adds a file that isn't assembly source (a TOML baseline, a note):
    /// its lines are read where a ref cites them, but define nothing.
    pub fn add_lines_only(&mut self, name: &str, text: &str) {
        self.files.insert(name.to_owned(), File::parse(name, text));
    }

    /// The value of a symbol: a constant, else a label's address if known.
    pub fn symbol(&self, name: &str) -> Option<i64> {
        self.symbol_depth(name, 0)
    }

    fn symbol_depth(&self, name: &str, depth: u32) -> Option<i64> {
        if depth > 32 {
            return None;
        }
        if let Some((_, _, e)) = self.consts.get(name) {
            return self.eval_depth(e, depth + 1);
        }
        self.addresses.get(name).copied()
    }

    /// Evaluates an expression over the constants. `None` when a name in
    /// it isn't a constant (a label with no address, a RAM variable written
    /// as a label).
    pub fn eval(&self, e: &Expr) -> Option<i64> {
        self.eval_depth(e, 0)
    }

    fn eval_depth(&self, e: &Expr, d: u32) -> Option<i64> {
        Some(match e {
            Expr::Num(n) => *n,
            Expr::Sym(s) => self.symbol_depth(s, d)?,
            Expr::Lo(x) => self.eval_depth(x, d)? & 0xFF,
            Expr::Hi(x) => (self.eval_depth(x, d)? >> 8) & 0xFF,
            Expr::Neg(x) => -self.eval_depth(x, d)?,
            Expr::Not(x) => !self.eval_depth(x, d)?,
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.eval_depth(a, d)?, self.eval_depth(b, d)?);
                match op {
                    '+' => a.checked_add(b)?,
                    '-' => a.checked_sub(b)?,
                    '*' => a.checked_mul(b)?,
                    '/' => a.checked_div(b)?,
                    '&' => a & b,
                    '|' => a | b,
                    '^' => a ^ b,
                    'l' => a.checked_shl(u32::try_from(b).ok()?)?,
                    'r' => a.checked_shr(u32::try_from(b).ok()?)?,
                    _ => return None,
                }
            }
        })
    }

    /// The name an operand refers to, when it's a plain name.
    pub fn name_of(e: &Expr) -> Option<&str> {
        match e {
            Expr::Sym(s) => Some(s),
            _ => None,
        }
    }

    /// The table that starts at `line` of `file`: the data rows from that
    /// line (when it holds data) or the next, up to the next global label
    /// or the first line that isn't data, a comment or empty. `None` when
    /// no data follows.
    pub fn table_at(&self, file: &str, line: usize) -> Option<Vec<Vec<Option<i64>>>> {
        let f = self.files.get(file)?;
        let mut rows = Vec::new();
        for (k, l) in f.lines.iter().enumerate().skip(line.checked_sub(1)?) {
            if k + 1 > line && l.global_label().is_some() {
                break;
            }
            match &l.stmt {
                Stmt::Data { width, items } => rows.push(
                    items
                        .iter()
                        .map(|i| {
                            i.as_ref()
                                .and_then(|e| self.eval(e))
                                .map(|v| if *width == 1 { v & 0xFF } else { v & 0xFFFF })
                        })
                        .collect(),
                ),
                Stmt::Empty | Stmt::Other if rows.is_empty() || matches!(l.stmt, Stmt::Empty) => {}
                _ => break,
            }
            if rows.len() >= 256 {
                break;
            }
        }
        (!rows.is_empty()).then_some(rows)
    }

    /// The table a global label names, by name.
    pub fn table(&self, label: &str) -> Option<Vec<Vec<Option<i64>>>> {
        let (f, l) = self.labels.get(label)?;
        self.table_at(f, *l)
    }

    /// How many lines name `label` in an operand or a data row, anywhere.
    pub fn references(&self, label: &str) -> usize {
        self.mentions.get(label).copied().unwrap_or(0)
    }
}

/// Every name in an expression.
pub fn symbols(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Num(_) => {}
        Expr::Sym(s) => out.push(s.clone()),
        Expr::Lo(x) | Expr::Hi(x) | Expr::Neg(x) | Expr::Not(x) => symbols(x, out),
        Expr::Bin(_, a, b) => {
            symbols(a, out);
            symbols(b, out);
        }
    }
}

/// Whether an expression names `sym`.
pub fn mentions(e: &Expr, sym: &str) -> bool {
    match e {
        Expr::Num(_) => false,
        Expr::Sym(s) => s == sym,
        Expr::Lo(x) | Expr::Hi(x) | Expr::Neg(x) | Expr::Not(x) => mentions(x, sym),
        Expr::Bin(_, a, b) => mentions(a, sym) || mentions(b, sym),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_of_each_dialect() {
        let l = parse_line("SPEED_CAP   = $2A   ; a made-up cap");
        assert_eq!(
            l.stmt,
            Stmt::Const {
                name: "SPEED_CAP".into(),
                value: Expr::Num(0x2A)
            }
        );
        let l = parse_line("HopTable:  .db $10, $f0, 3 ;comment");
        assert_eq!(l.label.as_deref(), Some("HopTable"));
        assert!(matches!(&l.stmt, Stmt::Data { width: 1, items } if items.len() == 3));
        let l = parse_line("@loop:  lda     #$83");
        assert_eq!(l.label.as_deref(), Some("@loop"));
        assert_eq!(l.global_label(), None);
        assert_eq!(
            l.stmt,
            Stmt::Op {
                mnemonic: "lda".into(),
                mode: Mode::Immediate,
                arg: Some(Expr::Num(0x83))
            }
        );
        let l = parse_line("        sta     ent_x_vel,x");
        assert!(matches!(
            l.stmt,
            Stmt::Op {
                mode: Mode::AddressX,
                ..
            }
        ));
        let l = parse_line("  lda (ptr),y");
        assert!(matches!(
            l.stmt,
            Stmt::Op {
                mode: Mode::IndirectY,
                ..
            }
        ));
        let l = parse_line("  asl");
        assert!(matches!(
            l.stmt,
            Stmt::Op {
                mode: Mode::Implied,
                ..
            }
        ));
        let l = parse_line(r#"  .byte "AB;C", $1A"#);
        assert!(
            matches!(&l.stmt, Stmt::Data { items, .. } if items.len() == 2 && items[0].is_none())
        );
        assert_eq!(parse_line(":  rts").label.as_deref(), Some(":"));
        assert_eq!(parse_line("  ; only a comment").stmt, Stmt::Empty);
    }

    #[test]
    fn expressions_and_constants() {
        let mut s = Source::default();
        s.add(
            "made-up.s",
            "BASE = $0300\nTOP = BASE + 4 * 2\nLOW = <(TOP | %1)\nNEG = -$05\nHEXH = 1Fh\n",
        );
        assert_eq!(s.symbol("TOP"), Some(0x308));
        assert_eq!(s.symbol("LOW"), Some(0x09));
        assert_eq!(s.symbol("NEG"), Some(-5));
        assert_eq!(s.symbol("HEXH"), Some(0x1F));
        assert_eq!(s.symbol("MISSING"), None);
    }

    #[test]
    fn a_table_runs_to_the_next_label() {
        let mut s = Source::default();
        s.add(
            "made-up.s",
            "Speeds:\n  .db $18, $28\n\n  .db $fc ; more\nNext:\n  .db 9\n",
        );
        assert_eq!(
            s.table("Speeds"),
            Some(vec![vec![Some(0x18), Some(0x28)], vec![Some(0xfc)]])
        );
        assert_eq!(s.table_at("made-up.s", 6), Some(vec![vec![Some(9)]]));
    }
}
