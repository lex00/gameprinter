//! Selectors and conditions: the strings `query`, `sample` and `run_until`
//! take.
//!
//! A selector is a subset of the model path grammar planned for Milestone 2
//! (docs/open-questions.md): dotted names, each optionally followed by an
//! index in brackets. An index is a number, a half-open range `a..b`, or `*`
//! for every element. Numbers are decimal, or hex after `0x` or `$`.
//!
//! ```text
//! selector  := segment ("." segment)*
//! segment   := name ("[" index "]")?
//! name      := [A-Za-z_][A-Za-z0-9_]*
//! index     := number | number ".." number | "*"    (a range ends after it starts)
//! number    := "-"? (digits | "0x" hex | "$" hex)
//! ```
//!
//! A condition compares selectors with numbers or with each other:
//!
//! ```text
//! condition := and ("||" and)*
//! and       := unary ("&&" unary)*
//! unary     := "!" unary | "(" condition ")" | compare
//! compare   := operand (("==" | "!=" | "<" | "<=" | ">" | ">=") operand)?
//! operand   := (selector | number) ("&" number)?
//! ```
//!
//! An operand without a comparison is true when it isn't zero, and `& mask`
//! keeps only the masked bits, so `ram[0x0770] == 1` and `ppu.status & 0x80`
//! are both conditions. Every selector in a condition must name exactly one
//! value. What names exist is up to each runtime.

use std::fmt;

/// A parsed selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: String,
    pub index: Option<Index>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    At(i64),
    /// From the first to before the second.
    Range(i64, i64),
    All,
}

impl Selector {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lexer = Lexer::new(text);
        let sel = lexer.selector()?;
        lexer.skip_space();
        if lexer.rest().is_empty() {
            Ok(sel)
        } else {
            Err(lexer.error("unexpected text after the selector"))
        }
    }

    /// The dotted name with no indexes, such as `cpu.pc`.
    pub fn name(&self) -> String {
        let names: Vec<&str> = self.segments.iter().map(|s| s.name.as_str()).collect();
        names.join(".")
    }

    /// The last segment's index, when no other segment has one. Raw state
    /// selectors index only their last segment.
    pub fn last_index(&self) -> Result<Option<Index>, String> {
        let (last, rest) = self.segments.split_last().expect("at least one segment");
        if rest.iter().any(|s| s.index.is_some()) {
            return Err(format!("{self}: only the last name can have an index here"));
        }
        Ok(last.index)
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", seg.name)?;
            match seg.index {
                None => {}
                Some(Index::At(n)) => write!(f, "[{}]", hex(n))?,
                Some(Index::Range(a, b)) => write!(f, "[{}..{}]", hex(a), hex(b))?,
                Some(Index::All) => write!(f, "[*]")?,
            }
        }
        Ok(())
    }
}

/// An index as entry paths write it: `0x` and four hex digits.
pub fn hex(n: i64) -> String {
    if n < 0 {
        format!("-{:#06x}", n.unsigned_abs())
    } else {
        format!("{n:#06x}")
    }
}

/// A parsed condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Or(Box<Condition>, Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
    Compare(Operand, Op, Operand),
    /// True when the operand isn't zero.
    Truthy(Operand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operand {
    pub atom: Atom,
    pub mask: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    Number(i64),
    Selector(Selector),
}

impl Condition {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lexer = Lexer::new(text);
        let cond = lexer.or()?;
        lexer.skip_space();
        if lexer.rest().is_empty() {
            Ok(cond)
        } else {
            Err(lexer.error("expected `&&`, `||` or the end of the condition"))
        }
    }

    /// Every selector the condition reads, in order.
    pub fn selectors(&self) -> Vec<&Selector> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a Selector>) {
        let mut operand = |o: &'a Operand| {
            if let Atom::Selector(s) = &o.atom {
                out.push(s);
            }
        };
        match self {
            Self::Or(a, b) | Self::And(a, b) => {
                a.collect(out);
                b.collect(out);
            }
            Self::Not(c) => c.collect(out),
            Self::Compare(a, _, b) => {
                operand(a);
                operand(b);
            }
            Self::Truthy(a) => operand(a),
        }
    }

    /// Evaluates the condition, reading each selector through `read`.
    pub fn eval(
        &self,
        read: &mut impl FnMut(&Selector) -> Result<i64, String>,
    ) -> Result<bool, String> {
        Ok(match self {
            Self::Or(a, b) => a.eval(read)? || b.eval(read)?,
            Self::And(a, b) => a.eval(read)? && b.eval(read)?,
            Self::Not(c) => !c.eval(read)?,
            Self::Compare(a, op, b) => {
                let (a, b) = (a.eval(read)?, b.eval(read)?);
                match op {
                    Op::Eq => a == b,
                    Op::Ne => a != b,
                    Op::Lt => a < b,
                    Op::Le => a <= b,
                    Op::Gt => a > b,
                    Op::Ge => a >= b,
                }
            }
            Self::Truthy(a) => a.eval(read)? != 0,
        })
    }
}

impl Operand {
    fn eval(&self, read: &mut impl FnMut(&Selector) -> Result<i64, String>) -> Result<i64, String> {
        let v = match &self.atom {
            Atom::Number(n) => *n,
            Atom::Selector(s) => read(s)?,
        };
        Ok(self.mask.map_or(v, |m| v & m))
    }
}

/// How deeply `!` and parentheses may nest, so a hostile condition can't
/// overflow the stack.
const MAX_DEPTH: usize = 64;

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    /// `!` and `(` currently open.
    depth: usize,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            depth: 0,
        }
    }

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

    /// Consumes `token` after any spaces, if it's next.
    fn eat(&mut self, token: &str) -> bool {
        self.skip_space();
        if self.rest().starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    /// Counts one more level of `!` or `(`.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error(&format!(
                "`!` and parentheses nest more than {MAX_DEPTH} deep"
            )));
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Condition, String> {
        let mut left = self.and()?;
        while self.eat("||") {
            left = Condition::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Condition, String> {
        let mut left = self.unary()?;
        while self.eat("&&") {
            left = Condition::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Condition, String> {
        self.skip_space();
        if self.rest().starts_with('!') && !self.rest().starts_with("!=") {
            self.pos += 1;
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            return Ok(Condition::Not(Box::new(inner)));
        }
        if self.eat("(") {
            self.enter()?;
            let inner = self.or()?;
            self.depth -= 1;
            if !self.eat(")") {
                return Err(self.error("expected `)`"));
            }
            return Ok(inner);
        }
        let left = self.operand()?;
        // Longest first, so `<=` isn't read as `<`.
        let ops = [
            ("==", Op::Eq),
            ("!=", Op::Ne),
            ("<=", Op::Le),
            (">=", Op::Ge),
            ("<", Op::Lt),
            (">", Op::Gt),
        ];
        for (token, op) in ops {
            if self.eat(token) {
                return Ok(Condition::Compare(left, op, self.operand()?));
            }
        }
        Ok(Condition::Truthy(left))
    }

    fn operand(&mut self) -> Result<Operand, String> {
        self.skip_space();
        let atom = match self.rest().chars().next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => Atom::Selector(self.selector()?),
            Some(c) if c.is_ascii_digit() || c == '$' || c == '-' => Atom::Number(self.number()?),
            _ => return Err(self.error("expected a selector or a number")),
        };
        self.skip_space();
        let mask = if self.rest().starts_with('&') && !self.rest().starts_with("&&") {
            self.pos += 1;
            self.skip_space();
            Some(self.number()?)
        } else {
            None
        };
        Ok(Operand { atom, mask })
    }

    fn selector(&mut self) -> Result<Selector, String> {
        self.skip_space();
        let mut segments = Vec::new();
        loop {
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
            } else {
                return Ok(Selector { segments });
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
        if self.rest().starts_with('*') {
            self.pos += 1;
            self.skip_space();
            return Ok(Index::All);
        }
        let first = self.number()?;
        self.skip_space();
        if self.rest().starts_with("..") {
            self.pos += 2;
            self.skip_space();
            let end = self.number()?;
            self.skip_space();
            if end <= first {
                return Err(self.error("a range must end after it starts"));
            }
            return Ok(Index::Range(first, end));
        }
        Ok(Index::At(first))
    }

    fn number(&mut self) -> Result<i64, String> {
        let negative = self.rest().starts_with('-');
        if negative {
            self.pos += 1;
        }
        let rest = self.rest();
        let (radix, skip) = if rest.starts_with("0x") || rest.starts_with("0X") {
            (16, 2)
        } else if rest.starts_with('$') {
            (16, 1)
        } else {
            (10, 0)
        };
        let digits = &rest[skip..];
        let len = digits
            .find(|c: char| !c.is_digit(radix) && c != '_')
            .unwrap_or(digits.len());
        let text: String = digits[..len].chars().filter(|c| *c != '_').collect();
        if text.is_empty() {
            return Err(self.error("expected a number"));
        }
        let n = i64::from_str_radix(&text, radix).map_err(|e| self.error(&e.to_string()))?;
        self.pos += skip + len;
        Ok(if negative { -n } else { n })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(text: &str) -> Selector {
        Selector::parse(text).unwrap()
    }

    #[test]
    fn parses_selectors() {
        let s = sel("ram[0x0770]");
        assert_eq!(s.name(), "ram");
        assert_eq!(s.last_index(), Ok(Some(Index::At(0x770))));
        assert_eq!(sel("ram[$770]"), s);
        assert_eq!(sel("ram[1904]"), s);
        assert_eq!(sel("cpu.pc").name(), "cpu.pc");
        assert_eq!(sel("cpu.pc").last_index(), Ok(None));
        assert_eq!(sel("oam[0..4]").last_index(), Ok(Some(Index::Range(0, 4))));
        assert_eq!(sel("palette[*]").last_index(), Ok(Some(Index::All)));
        assert_eq!(sel("apu.pulse1.timer.period").segments.len(), 4);
        let model = sel("entities.enemies[*].pos");
        assert_eq!(model.name(), "entities.enemies.pos");
        assert!(model.last_index().is_err());
    }

    #[test]
    fn prints_selectors_in_one_form() {
        assert_eq!(sel("ram[$770]").to_string(), "ram[0x0770]");
        assert_eq!(sel("oam[ 0 .. 4 ]").to_string(), "oam[0x0000..0x0004]");
        assert_eq!(sel("palette[*]").to_string(), "palette[*]");
        assert_eq!(sel("cpu.pc").to_string(), "cpu.pc");
    }

    #[test]
    fn refuses_bad_selectors() {
        for bad in [
            "",
            "1ram",
            "ram[",
            "ram[]",
            "ram[0x]",
            "ram[4..2]",
            "ram[4..4]",
            "ram]",
            "cpu.",
            "ram[0] x",
        ] {
            assert!(Selector::parse(bad).is_err(), "{bad:?}");
        }
        let e = Selector::parse("ram[0x0770").unwrap_err();
        assert!(e.contains("column 11"), "{e}");
    }

    fn eval(cond: &str) -> bool {
        Condition::parse(cond)
            .unwrap()
            .eval(&mut |s: &Selector| match s.to_string().as_str() {
                "ram[0x0770]" => Ok(1),
                "ppu.status" => Ok(0x80),
                "cpu.x" => Ok(5),
                other => Err(format!("no {other}")),
            })
            .unwrap()
    }

    #[test]
    fn evaluates_conditions() {
        assert!(eval("ram[0x0770] == 1"));
        assert!(eval("ram[$0770]==1"));
        assert!(!eval("ram[0x0770] != 1"));
        assert!(eval("ppu.status & 0x80"));
        assert!(!eval("ppu.status & 0x40"));
        assert!(eval("ppu.status & 0x80 == 0x80"));
        assert!(eval("cpu.x >= 5 && cpu.x < 6"));
        assert!(eval("cpu.x > 9 || ram[0x770] == 1"));
        assert!(eval("!(cpu.x > 9)"));
        assert!(eval("cpu.x > ram[0x0770]"));
        assert!(eval("cpu.x == 5 && (ram[0x0770] == 2 || ppu.status)"));
        assert!(eval("-1 < 0"));
        // && binds tighter than ||.
        assert!(eval("cpu.x == 5 || cpu.x == 6 && cpu.x == 7"));
    }

    #[test]
    fn lists_selectors_and_reports_errors() {
        let c = Condition::parse("ram[1] == 2 && (cpu.pc != ram[3] || !ppu.status)").unwrap();
        let names: Vec<String> = c.selectors().iter().map(|s| s.to_string()).collect();
        assert_eq!(
            names,
            ["ram[0x0001]", "cpu.pc", "ram[0x0003]", "ppu.status"]
        );
        for bad in [
            "",
            "ram[1] ==",
            "(cpu.x",
            "cpu.x == 1 extra",
            "== 1",
            "cpu.x & mask",
        ] {
            assert!(Condition::parse(bad).is_err(), "{bad:?}");
        }
        let missing = Condition::parse("nope == 1")
            .unwrap()
            .eval(&mut |_: &Selector| Err("no such field".to_owned()));
        assert_eq!(missing, Err("no such field".to_owned()));
    }

    #[test]
    fn limits_nesting() {
        let deep = |n: usize| format!("{}cpu.x{}", "(".repeat(n), ")".repeat(n));
        assert!(Condition::parse(&deep(MAX_DEPTH)).is_ok());
        assert!(Condition::parse(&deep(MAX_DEPTH + 1)).is_err());
        assert!(Condition::parse(&"!".repeat(MAX_DEPTH + 1)).is_err());
        // Far past the limit, it's still an error rather than a stack overflow.
        assert!(Condition::parse(&"!(".repeat(100_000)).is_err());
    }

    /// A small deterministic generator, so the fuzz test needs no crate and
    /// always runs the same cases.
    struct Fuzz(u64);

    impl Fuzz {
        fn new(seed: u64) -> Self {
            Self(seed | 1)
        }

        fn next(&mut self) -> u64 {
            // xorshift64
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// A string of grammar tokens and stray characters, including the
        /// largest and smallest numbers.
        fn text(&mut self) -> String {
            const TOKENS: &[&str] = &[
                "ram",
                "cpu",
                ".",
                "pc",
                "[",
                "]",
                "..",
                "*",
                "0x",
                "$",
                "-",
                "0",
                "1",
                "7ff",
                "0x7fffffffffffffff",
                "9223372036854775807",
                "-9223372036854775807",
                "18446744073709551615",
                "0xffffffffffffffff",
                "==",
                "!=",
                "<",
                "<=",
                ">",
                ">=",
                "&",
                "&&",
                "||",
                "!",
                "(",
                ")",
                " ",
                "_",
                "x",
                "\u{e9}",
            ];
            let n = self.next() % 12;
            (0..n)
                .map(|_| TOKENS[(self.next() % TOKENS.len() as u64) as usize])
                .collect()
        }
    }

    #[test]
    fn fuzzed_text_never_panics() {
        let mut fuzz = Fuzz::new(0x5eed);
        for _ in 0..20_000 {
            let text = fuzz.text();
            if let Ok(s) = Selector::parse(&text) {
                // What it prints parses back to the same selector.
                assert_eq!(Selector::parse(&s.to_string()), Ok(s.clone()), "{text:?}");
                if let Some(Index::Range(a, b)) = s.last_index().ok().flatten() {
                    assert!(a < b, "{text:?}");
                }
            }
            if let Ok(c) = Condition::parse(&text) {
                let _ = c.eval(&mut |_: &Selector| Ok(i64::MIN));
            }
        }
    }
}
