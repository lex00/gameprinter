#!/usr/bin/env python3
"""Turns MAME's z80.lst into Rust: crates/gp-arcade/src/z80/ops/.

    python3 -I crates/gp-arcade/tools/gen_z80.py crates/gp-arcade/mame/z80.lst \
        crates/gp-arcade/src/z80/ops

z80.lst (BSD-3-Clause, Juergen Buchmueller and Andre I. Holub, copied
unchanged into crates/gp-arcade/mame/) describes every Z80 instruction as
C++ statements broken into steps. MAME's own generator, z80make.py, turns
each step boundary into a `case` label so the CPU can stop between two
memory cycles of one instruction and carry on where it stopped in its next
timeslice. This script does the same for Rust. Rust has no jumps into the
middle of a block, so each instruction becomes a small state machine: a
`loop` over a `match` on the step number, where an `if` that holds a step
boundary is split into numbered blocks.

The C++ statements are translated by a small parser. Every value is
computed as an i64 and cut to the destination's width when stored, which
gives the same results as C's integer promotions for everything z80.lst
does (no multiplication, no shifts wider than 16 bits). Only the base Z80
is generated: entries tagged `r800:`, `z80n:` and `ncs800:` are left out,
as z80make.py does when run without a type.
"""

import re
import sys

# ---------------------------------------------------------------------------
# Reading z80.lst, the way z80make.py reads it (gen = None)
# ---------------------------------------------------------------------------


class Line:
    def __init__(self, text, indent):
        self.text = text.strip()
        self.indent = indent


def read_lst(path):
    macros = {}
    opcodes = []
    cur = None
    for raw in open(path):
        raw = raw.rstrip("\n")
        s = raw.strip()
        if not s:
            continue
        if s.startswith("#") and not s.startswith("#if") and not s.startswith("#endif"):
            continue
        indent = raw[: len(raw) - len(raw.lstrip())]
        if indent:
            if cur is not None:
                cur[1].append(Line(s, indent))
            continue
        tokens = s.split()
        if tokens[0] == "macro":
            names = tokens[1].split(":")
            arg = tokens[2] if len(tokens) > 2 else None
            body = []
            if len(names) == 2:
                cur = ("skip", body)
            else:
                macros[names[0]] = (arg, body)
                cur = ("macro", body)
        else:
            names = tokens[0].split(":")
            body = []
            if len(names) == 2:
                cur = ("skip", body)
            else:
                opcodes.append((names[0], body))
                cur = ("op", body)
    return macros, opcodes


def expand(lines, macros):
    out = []
    for line in lines:
        toks = line.text.split()
        times = 1
        text = line.text
        if len(toks) > 2 and toks[1] == "*":
            times = int(toks[0])
            toks = toks[2:]
            text = " ".join(toks)
        for _ in range(times):
            if toks[0] == "call":
                name = toks[1]
                arg = toks[2] if len(toks) > 2 else None
                if name not in macros:
                    raise SystemExit("macro not found: " + name)
                marg, body = macros[name]
                applied = [Line(l.text.replace(marg, arg) if marg else l.text, l.indent) for l in body]
                out.extend(expand(applied, macros))
            else:
                out.append(Line(text, line.indent))
    return out


# ---------------------------------------------------------------------------
# Tokens and a small C parser
# ---------------------------------------------------------------------------

TOKEN_RE = re.compile(
    r"\s*(?:(0x[0-9a-fA-F]+|\d+)[uUlL]*|([A-Za-z_][A-Za-z0-9_]*)|"
    r"(->|<<=|>>=|\+\+|--|<<|>>|<=|>=|==|!=|&&|\|\||\+=|-=|&=|\|=|\^=|::|[-+*/%&|^~!<>=?:;,(){}\[\].]))"
)


class Tok:
    def __init__(self, kind, val):
        self.kind = kind  # 'num', 'id', 'op', 'yield', 'access'
        self.val = val

    def __repr__(self):
        return "%s:%r" % (self.kind, self.val)


def tokenize(text):
    toks = []
    pos = 0
    text = text.strip()
    while pos < len(text):
        m = TOKEN_RE.match(text, pos)
        if not m or m.end() == pos:
            if text[pos:].strip() == "":
                break
            raise SystemExit("can't tokenize: %r at %r" % (text, text[pos:]))
        pos = m.end()
        if m.group(1) is not None:
            toks.append(Tok("num", int(m.group(1), 0)))
        elif m.group(2) is not None:
            toks.append(Tok("id", m.group(2)))
        else:
            toks.append(Tok("op", m.group(3)))
    return toks


def opcode_tokens(lines):
    """The token stream of one expanded opcode, with step boundaries as
    'yield' tokens and memory cycles as 'access' tokens."""
    toks = []
    skipping = False
    in_comment = False
    for line in lines:
        text = line.text
        if in_comment:
            if "*/" not in text:
                continue
            text = text[text.index("*/") + 2 :]
            in_comment = False
        text = re.sub(r"/\*.*?\*/", "", text)
        if "/*" in text:
            text = text[: text.index("/*")]
            in_comment = True
        text = re.sub(r"//.*$", "", text)
        # The daisy chain or the CPU's own IRQ callback gives the vector;
        # the bus knows which.
        text = text.replace(
            "m_tmp_irq_vector = (intf != nullptr) ? intf->z80daisy_irq_ack() : standard_irq_callback(0, m_pc.w);",
            "m_tmp_irq_vector = irq_ack();",
        )
        # Logging only; the strings would trip the tokenizer.
        text = re.sub(r"(LOGMASKED|logerror)\([^;]*\);", "", text).strip()
        if not text:
            continue
        if text.startswith("#if"):
            if "HAS_LDAIR_QUIRK" not in text:
                raise SystemExit("unexpected #if: " + text)
            skipping = True
            continue
        if text.startswith("#endif"):
            skipping = False
            continue
        if skipping:
            continue
        words = text.split()
        if words[0] == "+":
            toks.append(Tok("yield", " ".join(words[1:])))
        elif len(words) > 2 and words[1] == "!!":
            toks.append(Tok("access", (words[0], " ".join(words[2:]))))
        else:
            toks.extend(tokenize(text))
    return toks


class Parser:
    def __init__(self, toks):
        self.toks = toks
        self.i = 0

    def peek(self, k=0):
        j = self.i + k
        return self.toks[j] if j < len(self.toks) else Tok("eof", None)

    def next(self):
        t = self.peek()
        self.i += 1
        return t

    def is_op(self, val, k=0):
        t = self.peek(k)
        return t.kind == "op" and t.val == val

    def expect(self, val):
        t = self.next()
        if not (t.kind == "op" and t.val == val):
            raise SystemExit("expected %r, found %r (at %d in %r)" % (val, t, self.i, self.toks))
        return t

    # statements -----------------------------------------------------------

    def statements(self, until_brace=False):
        out = []
        while True:
            t = self.peek()
            if t.kind == "eof":
                if until_brace:
                    raise SystemExit("missing }")
                return out
            if until_brace and self.is_op("}"):
                self.next()
                return out
            s = self.statement()
            if s is not None:
                out.append(s)

    def statement(self):
        t = self.peek()
        if t.kind == "yield":
            self.next()
            return ("yield", parse_expr_text(t.val))
        if t.kind == "access":
            self.next()
            cycles, stmt = t.val
            body = Parser(tokenize(stmt)).statements()
            return ("access", parse_expr_text(cycles), body)
        if self.is_op("{"):
            self.next()
            return ("block", self.statements(until_brace=True))
        if self.is_op(";"):
            self.next()
            return None
        if t.kind == "id" and t.val == "if":
            self.next()
            self.expect("(")
            cond = self.expr()
            self.expect(")")
            then = self.statement()
            els = None
            if self.peek().kind == "id" and self.peek().val == "else":
                self.next()
                els = self.statement()
            return ("if", cond, as_list(then), as_list(els))
        if t.kind == "id" and t.val == "return":
            self.next()
            self.expect(";")
            return ("return",)
        if t.kind == "id" and t.val == "using":
            # `using std::swap;`
            while not self.is_op(";"):
                self.next()
            self.next()
            return None
        if t.kind == "id" and t.val in ("u8", "u16", "u32", "s8", "int", "unsigned"):
            self.next()
            name = self.next().val
            init = None
            if self.is_op("="):
                self.next()
                init = self.expr()
            self.expect(";")
            return ("decl", t.val, name, init)
        if t.kind == "id" and t.val == "device_z80daisy_interface":
            # device_z80daisy_interface *intf = daisy_get_irq_device();
            while not self.is_op(";"):
                self.next()
            self.next()
            return ("daisy_intf",)
        e = self.expr()
        self.expect(";")
        return ("expr", e)

    # expressions ----------------------------------------------------------

    def expr(self):
        return self.assign()

    def assign(self):
        lhs = self.ternary()
        t = self.peek()
        if t.kind == "op" and t.val in ("=", "+=", "-=", "&=", "|=", "^=", "<<=", ">>="):
            self.next()
            rhs = self.assign()
            return ("assign", t.val, lhs, rhs)
        return lhs

    def ternary(self):
        c = self.binary(0)
        if self.is_op("?"):
            self.next()
            a = self.assign()
            self.expect(":")
            b = self.assign()
            return ("?:", c, a, b)
        return c

    PREC = [
        ["||"], ["&&"], ["|"], ["^"], ["&"], ["==", "!="], ["<", "<=", ">", ">="],
        ["<<", ">>"], ["+", "-"], ["*", "/", "%"],
    ]

    def binary(self, level):
        if level == len(self.PREC):
            return self.unary()
        lhs = self.binary(level + 1)
        while self.peek().kind == "op" and self.peek().val in self.PREC[level]:
            op = self.next().val
            rhs = self.binary(level + 1)
            lhs = ("bin", op, lhs, rhs)
        return lhs

    CASTS = ("u8", "u16", "u32", "s8", "unsigned", "int", "s32", "uint8_t")

    def unary(self):
        t = self.peek()
        if t.kind == "op" and t.val in ("-", "!", "~"):
            self.next()
            return ("un", t.val, self.unary())
        if t.kind == "op" and t.val in ("++", "--"):
            self.next()
            return ("preinc", t.val, self.unary())
        if t.kind == "op" and t.val == "(" and self.peek(1).kind == "id" and self.peek(1).val in self.CASTS and self.is_op(")", 2):
            self.next()
            ty = self.next().val
            self.next()
            return ("cast", ty, self.unary())
        return self.postfix()

    def postfix(self):
        e = self.primary()
        while True:
            if self.is_op("("):
                self.next()
                args = []
                while not self.is_op(")"):
                    args.append(self.assign())
                    if self.is_op(","):
                        self.next()
                self.next()
                e = ("call", e, args)
            elif self.is_op("["):
                self.next()
                idx = self.expr()
                self.expect("]")
                e = ("index", e, idx)
            elif self.is_op(".") or self.is_op("->"):
                self.next()
                name = self.next().val
                e = ("member", e, name)
            elif self.is_op("++") or self.is_op("--"):
                op = self.next().val
                e = ("postinc", op, e)
            else:
                return e

    def primary(self):
        t = self.next()
        if t.kind == "num":
            return ("num", t.val)
        if t.kind == "id":
            if t.val == "std" and self.is_op("::"):
                self.next()
                return ("id", self.next().val)
            return ("id", t.val)
        if t.kind == "op" and t.val == "(":
            e = self.expr()
            self.expect(")")
            return e
        raise SystemExit("unexpected token %r in %r" % (t, self.toks))


def as_list(s):
    if s is None:
        return []
    if s[0] == "block":
        return s[1]
    return [s]


def parse_expr_text(text):
    p = Parser(tokenize(text))
    e = p.expr()
    return e


# ---------------------------------------------------------------------------
# Translation to Rust
# ---------------------------------------------------------------------------

# name -> (rust field, kind) where kind is "w" (u16 word), "h"/"l" (a byte
# of a word), "u8", "bool", "i32", "u32".
REGS = {
    "PC": ("pc", "w"), "PRVPC": ("prvpc", "w"), "SP": ("sp", "w"),
    "AF": ("af", "w"), "A": ("af", "h"), "F": ("af", "l"),
    "BC": ("bc", "w"), "B": ("bc", "h"), "C": ("bc", "l"),
    "DE": ("de", "w"), "D": ("de", "h"), "E": ("de", "l"),
    "HL": ("hl", "w"), "H": ("hl", "h"), "L": ("hl", "l"),
    "IX": ("ix", "w"), "HX": ("ix", "h"), "LX": ("ix", "l"),
    "IY": ("iy", "w"), "HY": ("iy", "h"), "LY": ("iy", "l"),
    "WZ": ("wz", "w"), "WZ_H": ("wz", "h"), "WZ_L": ("wz", "l"),
    "TADR": ("tadr", "w"), "TADR_H": ("tadr", "h"), "TADR_L": ("tadr", "l"),
    "TDAT": ("tdat", "w"), "TDAT_H": ("tdat", "h"), "TDAT_L": ("tdat", "l"), "TDAT8": ("tdat", "l"),
    "TDAT2": ("tdat2", "w"),
    "m_ea": ("ea", "w"),
    "Q": ("q", "u8"), "QT": ("qt", "u8"),
    "I": ("i", "u8"), "m_i": ("i", "u8"),
    "R": ("r", "u8"), "m_r": ("r", "u8"), "R2": ("r2", "u8"), "m_r2": ("r2", "u8"),
    "m_iff1": ("iff1", "u8"), "m_iff2": ("iff2", "u8"), "m_halt": ("halt", "u8"), "m_im": ("im", "u8"),
    "m_nmi_state": ("nmi_state", "u8"), "m_irq_state": ("irq_state", "u8"),
    "m_busack_state": ("busack_state", "u8"), "m_rtemp": ("rtemp", "u8"),
    "m_nmi_pending": ("nmi_pending", "bool"), "m_after_ei": ("after_ei", "bool"),
    "m_after_ldair": ("after_ldair", "bool"),
    "m_icount": ("icount", "i32"), "m_tmp_irq_vector": ("tmp_irq_vector", "i32"),
    "m_busrq_state": ("busrq_state", "i32"), "m_wait_state": ("wait_state", "i32"),
    "m_ref": ("m_ref", "u32"),
    "m_af": ("af", "w"), "m_af2": ("af2", "w"),
}

CONSTS = {
    "CF": 0x01, "NF": 0x02, "PF": 0x04, "VF": 0x04, "XF": 0x08, "HF": 0x10, "YF": 0x20,
    "ZF": 0x40, "SF": 0x80, "CLEAR_LINE": 0, "ASSERT_LINE": 1, "true": 1, "false": 0,
    "m_m1_cycles": 4, "m_memrq_cycles": 3, "m_iorq_cycles": 4,
}

TABLES = {"SZ", "SZ_BIT", "SZP", "SZHV_inc", "SZHV_dec", "SZHVC_add", "SZHVC_sub"}

# Callbacks MAME leaves unconnected on both of Tapper's Z80s, and logging.
DROPPED_CALLS = {
    "m_refresh_cb", "m_nomreq_cb", "m_branch_cb", "m_irqack_cb", "m_irqfetch_cb", "m_reti_cb",
    "m_busack_cb", "m_halt_cb",
    "LOGMASKED", "logerror", "debugger_wait_hook",
}

# Helper methods on Z80 that take and return i64 values.
VALUE_FNS = {
    "rlc", "rrc", "rl", "rr", "sla", "sra", "sll", "srl", "res", "set",
}
VOID_FNS = {
    "set_f", "add_a", "adc_a", "sub", "sbc_a", "neg", "daa", "and_a", "or_a", "xor_a", "cp",
    "rlca", "rrca", "rla", "rra", "exx", "ei", "halt", "leave_halt", "illegal_1", "illegal_2",
    "block_io_interrupted_flags", "bit", "bit_hl", "bit_xy",
}


class Ctx:
    def __init__(self):
        self.locals = {}  # name -> C type


def rd(name, ctx):
    if name in ctx.locals:
        return "l_" + name
    if name in REGS:
        field, kind = REGS[name]
        if kind == "w":
            return "(self.%s as i64)" % field
        if kind == "h":
            return "((self.%s >> 8) as i64)" % field
        if kind == "l":
            return "((self.%s & 0xff) as i64)" % field
        if kind == "bool":
            return "(self.%s as i64)" % field
        return "(self.%s as i64)" % field
    if name in CONSTS:
        return "%d" % CONSTS[name]
    raise SystemExit("unknown name " + name)


LOCAL_MASK = {"u8": "0xff", "u16": "0xffff", "u32": "0xffff_ffff", "unsigned": "0xffff_ffff"}


def wr(name, value, ctx):
    """A Rust statement storing i64 expression `value` in `name`."""
    if name in ctx.locals:
        ty = ctx.locals[name]
        if ty in LOCAL_MASK:
            return "l_%s = (%s) & %s;" % (name, value, LOCAL_MASK[ty])
        if ty == "s8":
            return "l_%s = (%s) as i8 as i64;" % (name, value)
        return "l_%s = (%s) as i32 as i64;" % (name, value)
    field, kind = REGS[name]
    if kind == "w":
        return "self.%s = (%s) as u16;" % (field, value)
    # The value first: it may call a helper that changes the other half
    # of the same register pair (A = sla(A) sets F).
    if kind == "h":
        return "{ let v = (%s) as u16; self.%s = (self.%s & 0x00ff) | ((v & 0xff) << 8); }" % (value, field, field)
    if kind == "l":
        return "{ let v = (%s) as u16; self.%s = (self.%s & 0xff00) | (v & 0xff); }" % (value, field, field)
    if kind == "u8":
        return "self.%s = (%s) as u8;" % (field, value)
    if kind == "bool":
        return "self.%s = (%s) != 0;" % (field, value)
    if kind == "i32":
        return "self.%s = (%s) as i32;" % (field, value)
    if kind == "u32":
        return "self.%s = (%s) as u32;" % (field, value)
    raise SystemExit("can't store " + name)


def lvalue_name(e):
    if e[0] == "id":
        return e[1]
    if e[0] == "member" and e[1] == ("member", ("id", "m_pc"), "b"):
        return {"l": "PC_L", "h": "PC_H"}[e[2]]
    if e[0] == "member" and e[1] == ("id", "m_pc") and e[2] == "w":
        return "PC"
    raise SystemExit("not an lvalue: %r" % (e,))


def as_int(code, kind):
    return "(%s as i64)" % code if kind == "bool" else code


def as_bool(code, kind):
    return code if kind == "bool" else "(%s != 0)" % code


def ex(e, ctx):
    """(rust code, kind) with kind 'int' (an i64) or 'bool'."""
    t = e[0]
    if t == "num":
        return ("%d" % e[1], "int")
    if t == "id":
        return (rd(e[1], ctx), "int")
    if t == "member":
        if e == ("member", ("id", "m_pc"), "w"):
            return (rd("PC", ctx), "int")
        raise SystemExit("member %r" % (e,))
    if t == "index":
        name = e[1][1]
        if name not in TABLES:
            raise SystemExit("index into " + name)
        idx, k = ex(e[2], ctx)
        return ("(tables().%s[(%s) as usize] as i64)" % (name.lower(), as_int(idx, k)), "int")
    if t == "cast":
        v, k = ex(e[2], ctx)
        v = as_int(v, k)
        ty = e[1]
        if ty in ("u8", "uint8_t"):
            return ("((%s) & 0xff)" % v, "int")
        if ty == "s8":
            return ("((%s) as i8 as i64)" % v, "int")
        if ty == "u16":
            return ("((%s) & 0xffff)" % v, "int")
        if ty in ("u32", "unsigned"):
            return ("((%s) & 0xffff_ffff)" % v, "int")
        return ("((%s) as i32 as i64)" % v, "int")
    if t == "un":
        v, k = ex(e[2], ctx)
        if e[1] == "!":
            return ("!%s" % as_bool(v, k), "bool")
        if e[1] == "-":
            return ("(-%s)" % as_int(v, k), "int")
        return ("(!%s)" % as_int(v, k), "int")
    if t == "bin":
        op = e[1]
        a, ka = ex(e[2], ctx)
        b, kb = ex(e[3], ctx)
        if op in ("&&", "||"):
            return ("(%s %s %s)" % (as_bool(a, ka), op, as_bool(b, kb)), "bool")
        if op in ("==", "!=", "<", "<=", ">", ">="):
            return ("(%s %s %s)" % (as_int(a, ka), op, as_int(b, kb)), "bool")
        if op in ("<<", ">>"):
            return ("(%s %s %s)" % (as_int(a, ka), op, as_int(b, kb)), "int")
        return ("(%s %s %s)" % (as_int(a, ka), op, as_int(b, kb)), "int")
    if t == "?:":
        c, kc = ex(e[1], ctx)
        a, ka = ex(e[2], ctx)
        b, kb = ex(e[3], ctx)
        return ("(if %s { %s } else { %s })" % (as_bool(c, kc), as_int(a, ka), as_int(b, kb)), "int")
    if t == "call":
        return call(e, ctx, value=True)
    if t == "preinc":
        name = lvalue_name(e[2])
        d = "+ 1" if e[1] == "++" else "- 1"
        return ("{ %s %s }" % (wr(name, "%s %s" % (rd(name, ctx), d), ctx), rd(name, ctx)), "int")
    raise SystemExit("expression %r" % (e,))


def call(e, ctx, value):
    f = e[1]
    args = e[2]
    if f[0] == "member":
        # m_io.read_interruptible(TADR) / m_io.write_interruptible(TADR, TDAT8)
        obj, meth = f[1], f[2]
        if obj == ("id", "m_io") and meth == "read_interruptible":
            a = as_int(*ex(args[0], ctx))
            return ("(bus.io_read((%s) as u16, &mut self.icount) as i64)" % a, "int")
        if obj == ("id", "m_io") and meth == "write_interruptible":
            a = as_int(*ex(args[0], ctx))
            v = as_int(*ex(args[1], ctx))
            return ("bus.io_write((%s) as u16, (%s) as u8, &mut self.icount)" % (a, v), "void")
        raise SystemExit("method call %r" % (e,))
    name = f[1]
    av = [as_int(*ex(a, ctx)) for a in args] if name not in ("inc", "dec", "swap") else None
    if name in ("data_read", "stack_read"):
        return ("(bus.read((%s) as u16, &mut self.icount) as i64)" % av[0], "int")
    if name in ("data_write", "stack_write"):
        return ("bus.write((%s) as u16, (%s) as u8, &mut self.icount)" % (av[0], av[1]), "void")
    if name in ("opcode_read", "arg_read"):
        return ("(bus.fetch(self.pc) as i64)", "int")
    if name == "debugger_instruction_hook":
        return ("bus.instruction(self)", "void")
    if name == "irq_ack":
        return ("(bus.irq_ack(&mut self.icount) as i64)", "int")
    if name == "daisy_call_reti_device":
        return ("bus.reti(&mut self.icount)", "void")
    if name in VALUE_FNS:
        return ("self.f_%s(%s)" % (name, ", ".join(av)), "int")
    if name in VOID_FNS:
        return ("self.f_%s(%s)" % (name, ", ".join(av)), "void")
    raise SystemExit("unknown function " + name)


def stmt_expr(e, ctx):
    """Rust statements for a C expression statement."""
    t = e[0]
    if t == "assign":
        op, lhs, rhs = e[1], e[2], e[3]
        name = lvalue_name(lhs)
        if op == "=" and rhs[0] == "assign":
            # a = b = v
            inner = stmt_expr(rhs, ctx)
            return inner + [wr(name, rd(lvalue_name(rhs[2]), ctx), ctx)]
        v = as_int(*ex(rhs, ctx))
        if op == "=":
            return [wr(name, v, ctx)]
        bop = op[:-1]
        return [
            "{ let v = %s; %s }" % (v, wr(name, "%s %s v" % (rd(name, ctx), bop), ctx))
        ]
    if t in ("postinc", "preinc"):
        name = lvalue_name(e[2])
        d = "+ 1" if e[1] == "++" else "- 1"
        return [wr(name, "%s %s" % (rd(name, ctx), d), ctx)]
    if t == "call":
        f = e[1]
        if f[0] == "id" and f[1] in DROPPED_CALLS:
            return []
        if f[0] == "id" and f[1] in ("inc", "dec"):
            name = lvalue_name(e[2][0])
            return ["{ let v = self.f_%s(%s); %s }" % (f[1], rd(name, ctx), wr(name, "v", ctx))]
        if f[0] == "id" and f[1] == "swap":
            a = lvalue_name(e[2][0])
            b = lvalue_name(e[2][1])
            return ["{ let t = %s; %s %s }" % (rd(a, ctx), wr(a, rd(b, ctx), ctx), wr(b, "t", ctx))]
        if f[0] == "member" and f[1][0] == "id" and f[1][1] in DROPPED_CALLS:
            return []
        code, kind = call(e, ctx, value=False)
        return [code + ";"]
    raise SystemExit("statement %r" % (e,))


def has_steps(stmts):
    for s in stmts:
        if s[0] in ("yield", "access"):
            return True
        if s[0] == "if" and (has_steps(s[2]) or has_steps(s[3])):
            return True
        if s[0] == "block" and has_steps(s[1]):
            return True
    return False


class Emitter:
    """Lowers one opcode's statements to numbered blocks."""

    def __init__(self):
        self.blocks = {}  # number -> list of lines
        self.count = 0
        self.ctx = Ctx()

    def new_block(self):
        n = self.count
        self.count += 1
        self.blocks[n] = []
        return n

    def straight(self, stmts, out, depth=0):
        """Statements without step boundaries, as nested Rust."""
        for s in stmts:
            t = s[0]
            if t == "expr":
                out.extend(stmt_expr(s[1], self.ctx))
            elif t == "decl":
                ty, name, init = s[1], s[2], s[3]
                self.ctx.locals[name] = ty
                v = as_int(*ex(init, self.ctx)) if init else "0"
                out.append("#[allow(unused_mut, unused_assignments)] let mut l_%s: i64 = 0;" % name)
                out.append(wr(name, v, self.ctx))
            elif t == "block":
                inner = []
                self.straight(s[1], inner, depth + 1)
                out.append("{")
                out.extend(inner)
                out.append("}")
            elif t == "if":
                c = as_bool(*ex(s[1], self.ctx))
                a = []
                self.straight(s[2], a, depth + 1)
                b = []
                self.straight(s[3], b, depth + 1)
                out.append("if %s {" % c)
                out.extend(a)
                if b:
                    out.append("} else {")
                    out.extend(b)
                out.append("}")
            elif t == "return":
                out.append("return;")
            elif t == "daisy_intf":
                pass
            else:
                raise SystemExit("straight: %r" % (s,))

    def lower(self, stmts, cur):
        """Appends stmts to block `cur`; returns the block that is current
        afterwards, or None when every path has returned."""
        for s in stmts:
            if cur is None:
                return None
            t = s[0]
            if t == "yield" or t == "access":
                cycles = as_int(*ex(s[1], self.ctx))
                if t == "access":
                    self.straight(s[2], self.blocks[cur])
                nxt = self.new_block()
                self.blocks[cur].append("self.icount -= %s;" % cycles)
                self.blocks[cur].append(
                    "if self.icount <= 0 { self.m_ref = (self.m_ref & 0xffff00) | %d; return; }" % nxt
                )
                self.blocks[cur].append("st = %d; continue;" % nxt)
                cur = nxt
            elif t == "if" and has_steps([s]):
                c = as_bool(*ex(s[1], self.ctx))
                then_b = self.new_block()
                else_b = self.new_block() if s[3] else None
                join = self.new_block()
                self.blocks[cur].append(
                    "if %s { st = %d; } else { st = %d; } continue;" % (c, then_b, else_b if else_b is not None else join)
                )
                end_t = self.lower(s[2], then_b)
                if end_t is not None:
                    self.blocks[end_t].append("st = %d; continue;" % join)
                if else_b is not None:
                    end_e = self.lower(s[3], else_b)
                    if end_e is not None:
                        self.blocks[end_e].append("st = %d; continue;" % join)
                cur = join
            elif t == "block" and has_steps([s]):
                cur = self.lower(s[1], cur)
            else:
                self.straight([s], self.blocks[cur])
        return cur


def emit_opcode(code, lines, macros):
    stmts = Parser(opcode_tokens(expand(lines, macros))).statements()
    em = Emitter()
    first = em.new_block()
    end = em.lower(stmts, first)
    if end is not None:
        em.blocks[end].append("self.m_ref = 0xffff00; return;")
    out = []
    fname = "op_%s" % code
    out.append("    #[allow(unused_mut, unused_variables, unreachable_code, clippy::all)]")
    out.append("    pub(in crate::z80) fn %s<B: Z80Bus>(&mut self, bus: &mut B) {" % fname)
    if em.count == 1:
        out.extend("        " + l for l in em.blocks[0])
    else:
        out.append("        let mut st = self.m_ref & 0xff;")
        out.append("        loop {")
        out.append("            match st {")
        for n in range(em.count):
            out.append("                %d => {" % n)
            body = em.blocks[n]
            out.extend("                    " + l for l in body)
            if not body or not (body[-1].endswith("continue;") or body[-1].endswith("return;")):
                # An empty join block after paths that all returned.
                out.append("                    return;")
            out.append("                }")
        out.append("                _ => unreachable!(),")
        out.append("            }")
        out.append("        }")
    out.append("    }")
    return fname, out


HEADER = [
    "// Generated by crates/gp-arcade/tools/gen_z80.py from crates/gp-arcade/mame/z80.lst",
    "// (MAME 0.276). Don't edit by hand: change the generator and run it again.",
    "//",
    "// z80.lst: license:BSD-3-Clause",
    "// copyright-holders:Juergen Buchmueller, Andrei I. Holub",
    "",
]


def main(argv):
    lst, out_dir = argv[1], argv[2]
    macros, opcodes = read_lst(lst)
    files = {}
    dispatch = []
    for code, body in opcodes:
        fname, out = emit_opcode(code, body, macros)
        files.setdefault(code[:2], []).extend(out + [""])
        dispatch.append("            0x%s => self.%s(bus)," % (code, fname))
    # One file per prefix, so no file is huge.
    mods = []
    for prefix, lines in files.items():
        mods.append("p" + prefix)
        text = HEADER + [
            "#![allow(unused_parens, unused_braces, unused_imports, clippy::all, clippy::pedantic)]",
            "",
            "use super::super::{Z80, Z80Bus, tables};",
            "",
            "impl Z80 {",
        ] + lines + ["}"]
        with open("%s/p%s.rs" % (out_dir, prefix), "w") as f:
            f.write("\n".join(text) + "\n")
    text = HEADER + ["//! The instructions, one function per opcode, by prefix.", ""]
    text += ["#[rustfmt::skip]\nmod %s;" % m for m in mods]
    text += [
        "",
        "use super::{Z80, Z80Bus};",
        "",
        "impl Z80 {",
        "    /// Runs the instruction step that `m_ref` names, as MAME's do_op.",
        "    pub(crate) fn do_op<B: Z80Bus>(&mut self, bus: &mut B) {",
        "        match (self.m_ref >> 8) & 0xffff {",
    ] + dispatch + [
        "            _ => unreachable!(),",
        "        }",
        "    }",
        "}",
    ]
    with open("%s/mod.rs" % out_dir, "w") as f:
        f.write("\n".join(text) + "\n")
    print("%d opcodes" % len(opcodes))


if __name__ == "__main__":
    main(sys.argv)
