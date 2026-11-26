//! Reading the printer's own output back: the check that the text says what the tree holds, which
//! running a function before and after the passes cannot make.
//!
//! The text and the function each come to one form, statements in their nesting over operator
//! trees with every reading taken off: a cast, a truth test and a pair of parentheses state no
//! operand, so `(int)x != 0` and `x` are one.
//! ⚠ What that form leaves out is not checked: a cast dropped where it counted reads the same.

use std::cell::Cell;
use std::fmt;

use super::ir::{
    BinOp, Callee, Const, Expr, ExprSugar, Head, Kind, Scalar, Stmt, StoreKind, UnOp, Var,
};
use super::names::Names;
use super::print::{callee_name, cells_of};
use super::tree;

/// The deepest that blocks or expressions nest, in text that is read and in a function that is compared.
const MAX_DEPTH: u32 = 128;

/// The statements and operands one function may come to, in its text and in what it holds, before it is not read back.
// ⚠ Each is held as a node of some tens of bytes until the two forms are compared: the bound is
// what one function may take of memory, and one past it is counted by who asked, never compared.
pub const MAX_NODES: usize = 1 << 22;

/// The marks a run of type arguments may come to: a longer one is no type.
const MAX_TYPE_MARKS: u32 = 256;

/// Why readable source did not read back, or did not agree with the function it was written from.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReparseError {
    #[error("line {line}: {what}")]
    Syntax { line: u32, what: String },
    #[error("line {line}: the text says `{text}` and the function holds `{held}`")]
    Differs {
        line: u32,
        text: String,
        held: String,
    },
    #[error("more than {limit} statements and operands: not read back")]
    TooLarge { limit: usize },
}

/// Reads readable source as the subset of C# the printer writes and returns the statements it holds; an error names the first line that is none of it.
pub fn parse(text: &str) -> Result<usize, ReparseError> {
    read(text, MAX_NODES, &mut |_, _| {})
}

/// Checks that `text`, written with no names list, says what `f` holds: the same statements in the same nesting, with the same calls, places and constants.
pub fn agrees(f: &tree::Function, text: &str) -> Result<(), ReparseError> {
    agrees_within(f, text, MAX_NODES)
}

/// [`agrees`], for a function and a text of no more than `limit` statements and operands each; `TooLarge` past it, with nothing compared.
pub fn agrees_within(f: &tree::Function, text: &str, limit: usize) -> Result<(), ReparseError> {
    let mut read_back = None;
    read(text, limit, &mut |name, body| {
        if name == f.head.name && read_back.is_none() {
            read_back = Some(body);
        }
    })?;
    let missing = |text: &str, held: &str| ReparseError::Differs {
        line: 0,
        text: text.to_owned(),
        held: held.to_owned(),
    };
    match (read_back, &f.head.incomplete) {
        (None, Some(_)) => Ok(()),
        (None, None) => Err(missing("no such function", &f.head.name)),
        (Some(_), Some(_)) => Err(missing(&f.head.name, "a function with no body")),
        (Some(body), None) => {
            let held = Held {
                head: &f.head,
                names: Names::default(),
                left: Cell::new(limit),
                limit,
            };
            compare(&held.nodes(&f.body, 0)?, &body)
        }
    }
}

/// How a constant was spelled, which says what a sign in front of it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lit {
    Int,
    F32,
    F64,
}

/// What an expression states once its readings are taken off.
#[derive(Clone, Debug)]
enum Sk {
    /// A constant's bits.
    Num(u64, Lit),
    /// A name as written: a variable, a callee, a type.
    Name(String),
    /// An operator or a form, by the mark the text writes it with, over its operands.
    Op(&'static str, Vec<Sk>),
    /// A call the printer writes by its name over every operand or, where a kind it reads its own way says so, in the second form `form` names.
    // ⚠ The operands are held once. Each form held whole doubles what a call nested in a call
    // comes to, and a chain of calls each on the result of the one before is then a power of two.
    Call {
        name: String,
        form: Form,
        args: Vec<Sk>,
    },
}

/// The second form a call is written in.
#[derive(Clone, Debug)]
enum Form {
    /// `receiver.method(rest)`, where the receiver is the first operand.
    Method(String),
    /// `l mark r` over the two operands. `not` says a `!` was taken in: the form by name is then under a `!`, and `mark` is the opposite comparison.
    Sign { mark: &'static str, not: bool },
}

impl fmt::Display for Sk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mark, items) = match self {
            Sk::Num(bits, _) => return write!(f, "{bits:#X}"),
            Sk::Name(name) => return f.write_str(name),
            Sk::Op(mark, items) => (*mark, items),
            Sk::Call { name, args, .. } => (name.as_str(), args),
        };
        write!(f, "{mark}(")?;
        for (n, item) in items.iter().enumerate() {
            if n > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{item}")?;
        }
        f.write_str(")")
    }
}

/// One statement as the text states it: its keyword, what its head holds and the blocks under it.
// An `if` is followed by its `else if` and `else` as statements of their own, so a chain of
// them is as flat here as it is in the text.
#[derive(Clone, Debug)]
struct Node {
    /// Empty for a simple statement.
    word: &'static str,
    head: Vec<Sk>,
    blocks: Vec<Vec<Node>>,
    /// The line of the text the statement starts on; 0 for one a function holds.
    line: u32,
}

impl Node {
    fn held(word: &'static str, head: Vec<Sk>, blocks: Vec<Vec<Node>>) -> Node {
        Node {
            word,
            head,
            blocks,
            line: 0,
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.word {
            "" => "statement",
            word => word,
        })?;
        for item in &self.head {
            write!(f, " {item}")?;
        }
        Ok(())
    }
}

fn deeper(depth: u32, line: u32) -> Result<u32, ReparseError> {
    match depth < MAX_DEPTH {
        true => Ok(depth + 1),
        false => Err(ReparseError::Syntax {
            line,
            what: format!("nested deeper than {MAX_DEPTH}"),
        }),
    }
}

fn num(value: u32) -> Sk {
    Sk::Num(u64::from(value), Lit::Int)
}

fn none() -> Sk {
    Sk::Op("none", Vec::new())
}

fn assign(place: Sk, value: Sk) -> Sk {
    Sk::Op("=", vec![place, value])
}

fn index_of(base: Sk, index: Sk) -> Sk {
    Sk::Op("[]", vec![base, index])
}

/// Items in parentheses: one alone is itself.
fn tuple(mut items: Vec<Sk>) -> Sk {
    match items.len() {
        1 => items.remove(0),
        _ => Sk::Op("{}", items),
    }
}

/// `new T(args)`.
fn made(ty: String, args: Vec<Sk>) -> Sk {
    let mut items = vec![Sk::Name(ty)];
    items.extend(args);
    Sk::Op("new", items)
}

/// `base.name`: one dotted name where the base is a name itself.
fn member(base: Sk, name: &str) -> Sk {
    match base {
        Sk::Name(path) => Sk::Name(format!("{path}.{name}")),
        other => Sk::Op(".", vec![other, Sk::Name(name.to_owned())]),
    }
}

/// A call; a reading of bits that is written as one is its operand.
fn called(callee: Sk, mut args: Vec<Sk>) -> Sk {
    if let (Sk::Name(name), 1) = (&callee, args.len()) {
        let reading = name
            .strip_prefix("as_")
            .is_some_and(|ty| Scalar::from_name(ty).is_some());
        if reading || name == "float_bits" {
            return args.remove(0);
        }
    }
    let mut items = vec![callee];
    items.append(&mut args);
    Sk::Op("()", items)
}

fn call(name: &str, args: Vec<Sk>) -> Sk {
    called(Sk::Name(name.to_owned()), args)
}

/// `e` as a test: a constant is whether any bit of it is set.
fn truth(e: Sk) -> Sk {
    match e {
        Sk::Num(bits, _) => Sk::Num(u64::from(bits != 0), Lit::Int),
        other => other,
    }
}

/// The comparison that is written for the negation of `mark`.
fn flipped(mark: &str) -> Option<&'static str> {
    Some(match mark {
        "<" => ">=",
        "<=" => ">",
        ">" => "<=",
        ">=" => "<",
        "==" => "!=",
        "!=" => "==",
        _ => return None,
    })
}

/// `!e`, in the one form `!(a < b)`, `a >= b` and `!!x` come to.
fn not(e: Sk) -> Sk {
    match e {
        Sk::Num(bits, _) => Sk::Num(u64::from(bits == 0), Lit::Int),
        // The sign takes the `!` in, as `!(a == b)` is written `a != b`; the form by name keeps it.
        Sk::Call {
            name,
            form: Form::Sign { mark, not: under },
            args,
        } => match flipped(mark) {
            Some(other) => Sk::Call {
                name,
                form: Form::Sign {
                    mark: other,
                    not: !under,
                },
                args,
            },
            None => {
                let form = Form::Sign { mark, not: under };
                Sk::Op("!", vec![Sk::Call { name, form, args }])
            }
        },
        Sk::Op("!", mut inner) if inner.len() == 1 => inner.remove(0),
        Sk::Op(mark, items) => match flipped(mark) {
            Some(other) => Sk::Op(other, items),
            None => Sk::Op("!", vec![Sk::Op(mark, items)]),
        },
        other => Sk::Op("!", vec![other]),
    }
}

/// `-e`: a constant is the constant with its sign turned.
fn neg(e: Sk) -> Sk {
    match e {
        Sk::Num(bits, Lit::Int) => Sk::Num(bits.wrapping_neg(), Lit::Int),
        Sk::Num(bits, Lit::F32) => Sk::Num(bits ^ (1 << 31), Lit::F32),
        Sk::Num(bits, Lit::F64) => Sk::Num(bits ^ (1 << 63), Lit::F64),
        other => Sk::Op("-", vec![other]),
    }
}

/// `l mark r`: a test against zero is the truth of `l`, and a chain of `&&` or `||` is one list.
fn binary(mark: &'static str, l: Sk, r: Sk) -> Sk {
    let zero = matches!(r, Sk::Num(0, _));
    match mark {
        "!=" if zero => truth(l),
        "==" if zero => not(l),
        "&&" | "||" => {
            let mut items = Vec::new();
            for side in [truth(l), truth(r)] {
                match side {
                    Sk::Op(under, inner) if under == mark => items.extend(inner),
                    other => items.push(other),
                }
            }
            Sk::Op(mark, items)
        }
        _ => Sk::Op(mark, vec![l, r]),
    }
}

/// The low bits of a cell read as `to`.
fn cast(to: Scalar, bits: u64) -> u64 {
    match to {
        Scalar::I8 => bits as i8 as i64 as u64,
        Scalar::U8 => u64::from(bits as u8),
        Scalar::I16 => bits as i16 as i64 as u64,
        Scalar::U16 => u64::from(bits as u16),
        Scalar::I32 => bits as i32 as i64 as u64,
        Scalar::U32 | Scalar::F32 => u64::from(bits as u32),
        Scalar::I64 | Scalar::U64 | Scalar::F64 => bits,
    }
}

/// Whether two constants are one: a narrow one widened with its sign or with zeroes is itself.
// ⚠ The text does not hold which of the two a widening was, so `0x8E7B3F2C` is both cells.
fn same_bits(a: u64, b: u64) -> bool {
    let widened = |x: u64, bits: u32| x >> bits == 0 || x >> bits == u64::MAX >> bits;
    let narrow = |bits: u32| {
        let mask = (1u64 << bits) - 1;
        a & mask == b & mask && widened(a, bits) && widened(b, bits)
    };
    a == b || [32, 16, 8].into_iter().any(narrow)
}

/// Whether `callee`, as the text writes it, is `method` of `receiver`.
fn receives(receiver: &Sk, method: &str, callee: &Sk) -> bool {
    match callee {
        // A receiver that is a name and its method are read as one dotted name.
        Sk::Name(path) => {
            let base = path
                .strip_suffix(method)
                .and_then(|base| base.strip_suffix('.'));
            base.is_some_and(|base| same(receiver, &Sk::Name(base.to_owned())))
        }
        Sk::Op(".", pair) => match pair.as_slice() {
            [base, Sk::Name(found)] => found == method && same(receiver, base),
            _ => false,
        },
        _ => false,
    }
}

/// Whether the text writes the call `name` over `args` by that name or in its second form.
// At most one form goes on to the operands: the two differ in the count of them or in the mark.
fn same_call(name: &str, form: &Form, args: &[Sk], text: &Sk) -> bool {
    let all = |held: &[Sk], text: &[Sk]| {
        held.len() == text.len() && held.iter().zip(text).all(|(h, t)| same(h, t))
    };
    let by_name = |text: &Sk| match text {
        Sk::Op("()", items) => match items.split_first() {
            Some((Sk::Name(callee), given)) => callee == name && all(args, given),
            _ => false,
        },
        _ => false,
    };
    match (form, text) {
        (Form::Method(method), Sk::Op("()", items)) => {
            let own = match (args.split_first(), items.split_first()) {
                (Some((receiver, passed)), Some((callee, given))) => {
                    passed.len() == given.len()
                        && receives(receiver, method, callee)
                        && all(passed, given)
                }
                _ => false,
            };
            own || by_name(text)
        }
        (Form::Method(_), _) => false,
        (Form::Sign { mark, not }, Sk::Op(found, items)) => {
            let under = match (*not, *found, items.as_slice()) {
                (true, "!", [inner]) => by_name(inner),
                (true, ..) => false,
                (false, ..) => by_name(text),
            };
            under || (found == mark && all(args, items))
        }
        (Form::Sign { .. }, _) => false,
    }
}

fn same(held: &Sk, text: &Sk) -> bool {
    match (held, text) {
        (Sk::Call { name, form, args }, text) => same_call(name, form, args, text),
        (Sk::Num(a, _), Sk::Num(b, _)) => same_bits(*a, *b),
        (Sk::Name(a), Sk::Name(b)) => a == b,
        (Sk::Op(a, xs), Sk::Op(b, ys)) => {
            a == b && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same(x, y))
        }
        _ => false,
    }
}

fn differs(line: u32, text: Option<&Node>, held: Option<&Node>) -> ReparseError {
    let shown = |node: Option<&Node>| node.map_or("nothing".to_owned(), Node::to_string);
    ReparseError::Differs {
        line,
        text: shown(text),
        held: shown(held),
    }
}

/// Holds the statements of the text against those of the function, block by block.
// An explicit stack: a body nests as deep as its listing does.
fn compare(held: &[Node], text: &[Node]) -> Result<(), ReparseError> {
    let mut work = vec![(held, text)];
    while let Some((held, text)) = work.pop() {
        for n in 0..held.len().max(text.len()) {
            let (Some(h), Some(t)) = (held.get(n), text.get(n)) else {
                let line = text.get(n).or(text.last()).map_or(0, |t| t.line);
                return Err(differs(line, text.get(n), held.get(n)));
            };
            let heads =
                h.head.len() == t.head.len() && h.head.iter().zip(&t.head).all(|(h, t)| same(h, t));
            if h.word != t.word || !heads || h.blocks.len() != t.blocks.len() {
                return Err(differs(t.line, Some(t), Some(h)));
            }
            for (h, t) in h.blocks.iter().zip(&t.blocks) {
                work.push((h.as_slice(), t.as_slice()));
            }
        }
    }
    Ok(())
}

/// Whether a case holds no statement, so that the labels after it are its own.
fn runs_on(case: &Node) -> bool {
    case.blocks.iter().all(Vec::is_empty)
}

/// The field a member name spells: `f_` and the byte offset in capital hex.
fn field(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("f_")?;
    let capital = |b: u8| b.is_ascii_digit() || (b'A'..=b'F').contains(&b);
    let hex = !digits.is_empty() && digits.bytes().all(capital);
    hex.then(|| u64::from_str_radix(digits, 16).ok()).flatten()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok<'a> {
    Ident(&'a str),
    Num(u64, Lit),
    Str(String),
    Punct(&'static str),
    End,
}

fn describe(tok: &Tok<'_>) -> String {
    match tok {
        Tok::Ident(word) => format!("`{word}`"),
        Tok::Num(..) => "a number".to_owned(),
        Tok::Str(_) => "a string".to_owned(),
        Tok::Punct(p) => format!("`{p}`"),
        Tok::End => "the end of the text".to_owned(),
    }
}

/// Every mark the readable form writes, a longer one before any it starts with.
const PUNCT: [&str; 36] = [
    "->", "++", "--", "&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "??", "+", "-", "*", "/",
    "%", "<", ">", "&", "|", "^", "!", "~", "=", "?", ":", ";", ",", ".", "(", ")", "{", "}", "[",
    "]",
];

#[derive(Clone, Debug)]
struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// The line `pos` is on, counted from 1.
    line: u32,
}

impl<'a> Lexer<'a> {
    fn refused(&self, what: &str) -> ReparseError {
        ReparseError::Syntax {
            line: self.line,
            what: what.to_owned(),
        }
    }

    /// The token at `pos`, which is moved past it; `End` for good once the text is out.
    fn token(&mut self) -> Result<Tok<'a>, ReparseError> {
        let bytes = self.src.as_bytes();
        let first = loop {
            match bytes.get(self.pos).copied() {
                None => return Ok(Tok::End),
                Some(b'\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                Some(b' ' | b'\t' | b'\r') => self.pos += 1,
                Some(b'/') if bytes.get(self.pos + 1) == Some(&b'/') => {
                    let rest = &bytes[self.pos..];
                    let end = rest.iter().position(|b| *b == b'\n');
                    self.pos += end.unwrap_or(rest.len());
                }
                Some(other) => break other,
            }
        };
        let start = self.pos;
        let named = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'@' | b'`');
        if named(first) && !first.is_ascii_digit() {
            let len = bytes[start..].iter().take_while(|b| named(**b)).count();
            self.pos = start + len;
            return Ok(Tok::Ident(&self.src[start..self.pos]));
        }
        if first.is_ascii_digit() {
            return self.number(start);
        }
        if first == b'"' {
            return self.string(start);
        }
        // A byte that is none of the marks may be the first of a longer character.
        let Some(rest) = self.src.get(start..) else {
            return Err(self.refused("a character the readable form does not write"));
        };
        for p in PUNCT {
            if rest.starts_with(p) {
                self.pos += p.len();
                return Ok(Tok::Punct(p));
            }
        }
        Err(self.refused("a character the readable form does not write"))
    }

    /// A number: decimal or hex, a fraction and an exponent, and the suffix that says its type.
    fn number(&mut self, start: usize) -> Result<Tok<'a>, ReparseError> {
        let rest = &self.src.as_bytes()[start..];
        let digits = |from: usize, hex: bool| {
            let digit = |b: &&u8| match hex {
                true => b.is_ascii_hexdigit(),
                false => b.is_ascii_digit(),
            };
            rest.iter().skip(from).take_while(digit).count()
        };
        let text = |from: usize, to: usize| std::str::from_utf8(&rest[from..to]).unwrap_or("");
        let unread = || self.refused("a number that does not read");
        let (mut n, bits, lit);
        if rest.starts_with(b"0x") {
            n = 2 + digits(2, true);
            bits = u64::from_str_radix(text(2, n), 16).map_err(|_| unread())?;
            lit = Lit::Int;
        } else {
            n = digits(0, false);
            let mut float = false;
            // A point belongs to the number only before a digit: `x[0].f_8` has none.
            if rest.get(n) == Some(&b'.') && rest.get(n + 1).is_some_and(u8::is_ascii_digit) {
                n += 1 + digits(n + 1, false);
                float = true;
            }
            if matches!(rest.get(n), Some(b'e' | b'E')) {
                let sign = usize::from(matches!(rest.get(n + 1), Some(b'+' | b'-')));
                let exponent = digits(n + 1 + sign, false);
                if exponent > 0 {
                    n += 1 + sign + exponent;
                    float = true;
                }
            }
            let literal = text(0, n);
            match rest.get(n) {
                Some(b'f' | b'F') => {
                    let value: f32 = literal.parse().map_err(|_| unread())?;
                    (bits, lit) = (u64::from(value.to_bits()), Lit::F32);
                    n += 1;
                }
                Some(b'd' | b'D') => {
                    let value: f64 = literal.parse().map_err(|_| unread())?;
                    (bits, lit) = (value.to_bits(), Lit::F64);
                    n += 1;
                }
                _ if float => {
                    let value: f64 = literal.parse().map_err(|_| unread())?;
                    (bits, lit) = (value.to_bits(), Lit::F64);
                }
                _ => {
                    bits = literal.parse::<u64>().map_err(|_| unread())?;
                    lit = Lit::Int;
                }
            }
        }
        if lit == Lit::Int {
            let suffix = [&b"UL"[..], &b"U"[..], &b"L"[..]];
            let found = suffix.into_iter().find(|s| rest[n..].starts_with(s));
            n += found.map_or(0, <[u8]>::len);
        }
        if rest
            .get(n)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return Err(unread());
        }
        self.pos = start + n;
        Ok(Tok::Num(bits, lit))
    }

    /// A string, with the three escapes the printer writes.
    fn string(&mut self, start: usize) -> Result<Tok<'a>, ReparseError> {
        let mut out = String::new();
        let mut chars = self.src[start + 1..].char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' => {
                    self.pos = start + at + 2;
                    return Ok(Tok::Str(out));
                }
                '\n' => break,
                '\\' => match chars.next() {
                    Some((_, 'u')) => {
                        let hex: String = chars.by_ref().take(4).map(|(_, c)| c).collect();
                        let code = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                        out.push(code.ok_or_else(|| self.refused("a malformed escape"))?);
                    }
                    Some((_, c @ ('"' | '\\'))) => out.push(c),
                    _ => return Err(self.refused("an escape the readable form does not write")),
                },
                c => out.push(c),
            }
        }
        Err(self.refused("a string with no end"))
    }
}

/// A callee's name out of the quotes the printer puts round one that is no C# name.
fn unquoted(name: String) -> String {
    if !name.starts_with('"') {
        return name;
    }
    let read = Lexer {
        src: &name,
        pos: 0,
        line: 0,
    }
    .token();
    let inner = match read {
        Ok(Tok::Str(text)) => Some(text),
        _ => None,
    };
    inner.unwrap_or(name)
}

#[derive(Clone)]
struct Parser<'a> {
    lex: Lexer<'a>,
    tok: Tok<'a>,
    /// The line `tok` is on.
    line: u32,
    /// The simple statements read.
    statements: usize,
    /// The statements and operands the function being read may still come to, of `limit`.
    left: usize,
    limit: usize,
}

/// Reads every function of `text`, each of no more than `limit` statements and operands, hands each to `each` and returns the simple statements read.
// One function's statements are held at a time.
fn read(
    text: &str,
    limit: usize,
    each: &mut dyn FnMut(String, Vec<Node>),
) -> Result<usize, ReparseError> {
    let mut parser = Parser {
        lex: Lexer {
            src: text,
            pos: 0,
            line: 1,
        },
        tok: Tok::End,
        line: 1,
        statements: 0,
        left: limit,
        limit,
    };
    parser.bump()?;
    while parser.tok != Tok::End {
        parser.left = limit;
        let (name, body) = parser.function()?;
        each(name, body);
    }
    Ok(parser.statements)
}

impl<'a> Parser<'a> {
    /// Moves to the next token and returns the one left.
    fn bump(&mut self) -> Result<Tok<'a>, ReparseError> {
        let next = self.lex.token()?;
        self.line = self.lex.line;
        Ok(std::mem::replace(&mut self.tok, next))
    }

    /// Takes one statement or operand of what the function may come to.
    fn spend(&mut self) -> Result<(), ReparseError> {
        match self.left.checked_sub(1) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => Err(ReparseError::TooLarge { limit: self.limit }),
        }
    }

    fn is(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::Punct(q) if *q == p)
    }

    fn word(&self, w: &str) -> bool {
        matches!(&self.tok, Tok::Ident(q) if *q == w)
    }

    fn eat(&mut self, p: &str) -> Result<bool, ReparseError> {
        let found = self.is(p);
        if found {
            self.bump()?;
        }
        Ok(found)
    }

    fn refused(&self, what: String) -> ReparseError {
        ReparseError::Syntax {
            line: self.line,
            what,
        }
    }

    fn unexpected(&self, wanted: &str) -> ReparseError {
        self.refused(format!("{wanted} expected, not {}", describe(&self.tok)))
    }

    fn expect(&mut self, p: &str) -> Result<(), ReparseError> {
        match self.eat(p)? {
            true => Ok(()),
            false => Err(self.unexpected(&format!("`{p}`"))),
        }
    }

    fn expect_word(&mut self, w: &str) -> Result<(), ReparseError> {
        match self.word(w) {
            true => self.bump().map(|_| ()),
            false => Err(self.unexpected(&format!("`{w}`"))),
        }
    }

    fn ident(&mut self) -> Result<&'a str, ReparseError> {
        let name = match &self.tok {
            Tok::Ident(name) => *name,
            _ => return Err(self.unexpected("a name")),
        };
        self.bump()?;
        Ok(name)
    }

    /// Whether `p` is the token after this one.
    fn next_is(&self, p: &str) -> bool {
        let mut ahead = self.clone();
        ahead.bump().is_ok() && ahead.is(p)
    }

    /// `<..>` after a name, taken into `text`.
    // ⚠ Asked ahead at every `<` after a name, so a comparison in a long list of names is read
    // as far as this goes: without the bound each `<` of the list would read the list again.
    fn generic(&mut self, text: &mut String) -> Result<(), ReparseError> {
        let mut open = 0u32;
        let mut marks = 0u32;
        loop {
            marks += 1;
            if marks > MAX_TYPE_MARKS {
                return Err(self.unexpected("`>`"));
            }
            match &self.tok {
                Tok::Punct("<") => open += 1,
                Tok::Punct(">") if open >= 1 => open -= 1,
                Tok::Punct(">>") if open >= 2 => open -= 2,
                Tok::Punct("." | "," | "[" | "]") | Tok::Ident(_) if open >= 1 => {}
                _ => return Err(self.unexpected("`>`")),
            }
            match self.bump()? {
                Tok::Punct(p) => text.push_str(p),
                Tok::Ident(word) => text.push_str(word),
                _ => {}
            }
            if open == 0 {
                return Ok(());
            }
        }
    }

    /// The name of a type: a quoted one, or names joined by points with their type arguments.
    fn type_name(&mut self) -> Result<String, ReparseError> {
        if let Tok::Str(name) = &self.tok {
            let name = name.clone();
            self.bump()?;
            return Ok(name);
        }
        let mut text = self.ident()?.to_owned();
        loop {
            if self.eat(".")? {
                text.push('.');
                text.push_str(self.ident()?);
            } else if self.is("<") {
                self.generic(&mut text)?;
            } else {
                return Ok(text);
            }
        }
    }

    /// A type as a declaration or a cast names it: `var[2]`, `int[]`, `void*`.
    fn ty(&mut self) -> Result<String, ReparseError> {
        let mut text = self.type_name()?;
        loop {
            if self.eat("[")? {
                text.push('[');
                if let Tok::Num(cells, _) = &self.tok {
                    text.push_str(&cells.to_string());
                    self.bump()?;
                }
                self.expect("]")?;
                text.push(']');
            } else if self.eat("*")? {
                text.push('*');
            } else {
                return Ok(text);
            }
        }
    }

    /// Whether a type and the name it declares start here: `int x =`, `var[2] pair;`, `void* a, b;`.
    fn declaration_ahead(&self) -> bool {
        let mut ahead = self.clone();
        let typed = ahead.ty().is_ok() && matches!(ahead.tok, Tok::Ident(_));
        typed && ahead.bump().is_ok() && (ahead.is("=") || ahead.is(",") || ahead.is(";"))
    }

    /// Whether the `(` here opens a cast: `(int)`, `(var[2])`.
    // ⚠ Only the types the printer casts to: `(x)` round a name is that name.
    fn cast_ahead(&self) -> bool {
        let keyword = |word: &str| word == "var" || Scalar::ALL.iter().any(|s| s.keyword() == word);
        let mut ahead = self.clone();
        let typed = ahead.bump().is_ok() && matches!(ahead.tok, Tok::Ident(word) if keyword(word));
        typed && ahead.ty().is_ok() && ahead.is(")")
    }

    /// Whether the `<` here opens type arguments that a `(` follows, as in `box<type_1>(x)`.
    fn generic_call_ahead(&self) -> bool {
        let mut ahead = self.clone();
        ahead.generic(&mut String::new()).is_ok() && ahead.is("(")
    }

    /// One function: its head, then its body.
    fn function(&mut self) -> Result<(String, Vec<Node>), ReparseError> {
        self.ty()?;
        let name = self.ident()?.to_owned();
        self.expect("(")?;
        while !self.is(")") {
            self.ty()?;
            self.ident()?;
            if !self.eat(",")? {
                break;
            }
        }
        self.expect(")")?;
        Ok((name, self.block(0)?))
    }

    /// `{ .. }`.
    fn block(&mut self, depth: u32) -> Result<Vec<Node>, ReparseError> {
        let d = deeper(depth, self.line)?;
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.is("}") {
            self.statement(&mut out, d)?;
        }
        self.bump()?;
        Ok(out)
    }

    /// A keyword, a head in parentheses and a block.
    fn headed(&mut self, word: &'static str, line: u32, depth: u32) -> Result<Node, ReparseError> {
        self.bump()?;
        let head = vec![self.test(depth)?];
        let blocks = vec![self.block(depth)?];
        Ok(Node {
            word,
            head,
            blocks,
            line,
        })
    }

    /// A keyword, what may follow it and a semicolon.
    fn worded(&mut self, word: &'static str, line: u32, depth: u32) -> Result<Node, ReparseError> {
        self.bump()?;
        let head = match self.is(";") {
            true => Vec::new(),
            false => vec![self.expr(depth)?],
        };
        self.expect(";")?;
        Ok(Node {
            word,
            head,
            blocks: Vec::new(),
            line,
        })
    }

    /// A keyword and a block.
    fn blocked(
        &mut self,
        word: &'static str,
        head: Vec<Sk>,
        line: u32,
        depth: u32,
    ) -> Result<Node, ReparseError> {
        let blocks = vec![self.block(depth)?];
        Ok(Node {
            word,
            head,
            blocks,
            line,
        })
    }

    fn statement(&mut self, out: &mut Vec<Node>, depth: u32) -> Result<(), ReparseError> {
        self.spend()?;
        let line = self.line;
        let word = match &self.tok {
            Tok::Ident(word) => *word,
            _ => "",
        };
        let node = match word {
            "if" => {
                out.push(self.headed("if", line, depth)?);
                // `else if` and `else` follow as statements of their own.
                while self.word("else") {
                    let line = self.line;
                    self.bump()?;
                    if self.word("if") {
                        out.push(self.headed("else if", line, depth)?);
                    } else {
                        out.push(self.blocked("else", Vec::new(), line, depth)?);
                        break;
                    }
                }
                return Ok(());
            }
            "while" => self.headed("while", line, depth)?,
            "do" => {
                self.bump()?;
                let blocks = vec![self.block(depth)?];
                self.expect_word("while")?;
                let head = vec![self.test(depth)?];
                self.expect(";")?;
                Node {
                    word: "do",
                    head,
                    blocks,
                    line,
                }
            }
            "for" => {
                self.bump()?;
                self.expect("(")?;
                let init = match self.is(";") {
                    true => none(),
                    false => self.simple(depth)?.unwrap_or_else(none),
                };
                self.expect(";")?;
                let cond = match self.is(";") {
                    true => none(),
                    false => truth(self.expr(depth)?),
                };
                self.expect(";")?;
                let step = match self.is(")") {
                    true => none(),
                    false => self.simple(depth)?.unwrap_or_else(none),
                };
                self.expect(")")?;
                self.blocked("for", vec![init, cond, step], line, depth)?
            }
            "switch" => {
                self.bump()?;
                self.expect("(")?;
                let head = vec![self.expr(depth)?];
                self.expect(")")?;
                let blocks = vec![self.cases(depth)?];
                Node {
                    word: "switch",
                    head,
                    blocks,
                    line,
                }
            }
            "break" => self.worded("break", line, depth)?,
            "continue" => self.worded("continue", line, depth)?,
            "unreachable" => self.worded("unreachable", line, depth)?,
            "return" => self.worded("return", line, depth)?,
            "await" => self.worded("await", line, depth)?,
            "throw" => self.worded("throw", line, depth)?,
            "goto" => {
                self.bump()?;
                let head = vec![Sk::Name(self.ident()?.to_owned())];
                self.expect(";")?;
                Node {
                    word: "goto",
                    head,
                    blocks: Vec::new(),
                    line,
                }
            }
            "yield" => {
                self.bump()?;
                self.expect_word("return")?;
                self.expect(";")?;
                Node {
                    word: "yield",
                    head: Vec::new(),
                    blocks: Vec::new(),
                    line,
                }
            }
            "foreach" => {
                self.bump()?;
                self.expect("(")?;
                self.ty()?;
                let item = Sk::Name(self.ident()?.to_owned());
                self.expect_word("in")?;
                let source = self.expr(depth)?;
                self.expect(")")?;
                self.blocked("foreach", vec![item, source], line, depth)?
            }
            "using" | "lock" => {
                let word = if word == "using" { "using" } else { "lock" };
                self.bump()?;
                self.expect("(")?;
                let head = match self.is(")") {
                    true => Vec::new(),
                    false => self.simple(depth)?.into_iter().collect(),
                };
                self.expect(")")?;
                self.blocked(word, head, line, depth)?
            }
            "try" => {
                self.bump()?;
                self.blocked("try", Vec::new(), line, depth)?
            }
            "catch" => {
                self.bump()?;
                let mut head = Vec::new();
                if self.eat("(")? {
                    head.push(Sk::Name(self.ty()?));
                    if !self.is(")") {
                        head.push(Sk::Name(self.ident()?.to_owned()));
                    }
                    self.expect(")")?;
                }
                self.blocked("catch", head, line, depth)?
            }
            "finally" => {
                self.bump()?;
                self.blocked("finally", Vec::new(), line, depth)?
            }
            // `label_000040: ;`
            label if !label.is_empty() && self.next_is(":") => {
                self.bump()?;
                self.bump()?;
                self.expect(";")?;
                Node {
                    word: "label",
                    head: vec![Sk::Name(label.to_owned())],
                    blocks: Vec::new(),
                    line,
                }
            }
            _ => {
                let stated = self.simple(depth)?;
                self.expect(";")?;
                // A declaration with no value states nothing that runs.
                let Some(stated) = stated else {
                    return Ok(());
                };
                Node {
                    word: "",
                    head: vec![stated],
                    blocks: Vec::new(),
                    line,
                }
            }
        };
        out.push(node);
        Ok(())
    }

    /// The cases of a switch, each under the labels that lead to it.
    fn cases(&mut self, depth: u32) -> Result<Vec<Node>, ReparseError> {
        let d = deeper(depth, self.line)?;
        self.expect("{")?;
        let mut cases: Vec<Node> = Vec::new();
        while !self.is("}") {
            let line = self.line;
            let label = if self.word("case") {
                self.bump()?;
                let negative = self.eat("-")?;
                let value = match self.bump()? {
                    Tok::Num(bits, Lit::Int) => bits,
                    other => {
                        let what = format!("a case's number expected, not {}", describe(&other));
                        return Err(self.refused(what));
                    }
                };
                self.expect(":")?;
                let value = if negative {
                    value.wrapping_neg()
                } else {
                    value
                };
                Some(Sk::Num(value, Lit::Int))
            } else if self.word("default") && self.next_is(":") {
                self.bump()?;
                self.bump()?;
                Some(Sk::Name("default".to_owned()))
            } else {
                None
            };
            let Some(label) = label else {
                let body = cases.last_mut().and_then(|last| last.blocks.first_mut());
                match body {
                    Some(body) => self.statement(body, d)?,
                    None => return Err(self.unexpected("`case`")),
                }
                continue;
            };
            // A case with no statement runs into the next: its labels are that one's.
            let joins = cases.last().is_some_and(runs_on);
            match cases.last_mut() {
                Some(last) if joins => last.head.push(label),
                _ => cases.push(Node {
                    word: "case",
                    head: vec![label],
                    blocks: vec![Vec::new()],
                    line,
                }),
            }
        }
        self.bump()?;
        Ok(cases)
    }

    /// A simple statement without its semicolon; `None` for a declaration that assigns nothing.
    fn simple(&mut self, depth: u32) -> Result<Option<Sk>, ReparseError> {
        if self.declaration_ahead() {
            self.ty()?;
            let name = self.ident()?.to_owned();
            if self.eat("=")? {
                self.statements += 1;
                return Ok(Some(assign(Sk::Name(name), self.expr(depth)?)));
            }
            while self.eat(",")? {
                self.ident()?;
            }
            return Ok(None);
        }
        self.statements += 1;
        let target = self.expr(depth)?;
        Ok(Some(match self.eat("=")? {
            true => assign(target, self.expr(depth)?),
            false => target,
        }))
    }

    /// A condition in its parentheses.
    fn test(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        self.expect("(")?;
        let cond = self.expr(depth)?;
        self.expect(")")?;
        Ok(truth(cond))
    }

    fn expr(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        let d = deeper(depth, self.line)?;
        let cond = self.binary(2, d)?;
        if self.eat("??")? {
            return Ok(Sk::Op("??", vec![cond, self.expr(d)?]));
        }
        if !self.eat("?")? {
            return Ok(cond);
        }
        let then = self.expr(d)?;
        self.expect(":")?;
        let els = self.expr(d)?;
        Ok(Sk::Op("?:", vec![truth(cond), then, els]))
    }

    /// Operators of C#'s strength `min` and above; `as` binds as a comparison does.
    fn binary(&mut self, min: u8, depth: u32) -> Result<Sk, ReparseError> {
        let mut left = self.unary(depth)?;
        loop {
            let op = match &self.tok {
                Tok::Punct(p) => BinOp::from_symbol(p),
                _ => None,
            };
            let strength = match op {
                Some(op) => op.precedence(),
                None if self.word("as") => 8,
                None => return Ok(left),
            };
            if strength < min {
                return Ok(left);
            }
            self.bump()?;
            left = match op {
                // No deeper: the strength rises with each operand, so this ends by itself.
                Some(op) => binary(op.symbol(), left, self.binary(strength + 1, depth)?),
                None => Sk::Op("as", vec![left, Sk::Name(self.ty()?)]),
            };
        }
    }

    fn unary(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        let mark = match &self.tok {
            Tok::Punct(p @ ("!" | "-" | "~" | "&" | "*")) => *p,
            Tok::Ident("out" | "ref") => "&",
            Tok::Punct("(") if self.cast_ahead() => "(",
            _ => return self.postfix(depth),
        };
        let d = deeper(depth, self.line)?;
        self.bump()?;
        if mark == "(" {
            self.ty()?;
            self.expect(")")?;
            return self.unary(d);
        }
        let e = self.unary(d)?;
        Ok(match mark {
            "!" => not(e),
            "-" => neg(e),
            mark => Sk::Op(mark, vec![e]),
        })
    }

    fn postfix(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        let mut e = self.primary(depth)?;
        loop {
            if self.eat("(")? {
                e = called(e, self.list(")", depth)?);
            } else if self.eat("[")? {
                let index = self.expr(depth)?;
                self.expect("]")?;
                e = index_of(e, index);
            } else if self.eat(".")? {
                let name = self.ident()?;
                e = match field(name) {
                    Some(offset) => Sk::Op(".f", vec![e, Sk::Num(offset, Lit::Int)]),
                    None => member(e, name),
                };
            } else if self.eat("->")? {
                let name = self.ident()?;
                let Some(offset) = field(name) else {
                    return Err(self.refused(format!("`->{name}` is no field")));
                };
                e = Sk::Op("->", vec![e, Sk::Num(offset, Lit::Int)]);
            } else if self.is("++") || self.is("--") {
                let mark = if self.is("++") { "++" } else { "--" };
                self.bump()?;
                e = Sk::Op(mark, vec![e]);
            } else if self.is("<") && matches!(e, Sk::Name(_)) && self.generic_call_ahead() {
                if let Sk::Name(name) = &mut e {
                    self.generic(name)?;
                }
            } else {
                return Ok(e);
            }
        }
    }

    /// Expressions between commas up to `close`, which is taken.
    fn list(&mut self, close: &str, depth: u32) -> Result<Vec<Sk>, ReparseError> {
        let mut items = Vec::new();
        while !self.is(close) {
            items.push(self.expr(depth)?);
            if !self.eat(",")? {
                break;
            }
        }
        self.expect(close)?;
        Ok(items)
    }

    fn primary(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        self.spend()?;
        let line = self.line;
        Ok(match self.bump()? {
            Tok::Num(bits, lit) => Sk::Num(bits, lit),
            Tok::Str(text) => Sk::Name(text),
            Tok::Punct("(") => tuple(self.list(")", depth)?),
            Tok::Punct("{") => Sk::Op("{}", self.list("}", depth)?),
            Tok::Ident("true") => Sk::Num(1, Lit::Int),
            Tok::Ident("false" | "null") => Sk::Num(0, Lit::Int),
            Tok::Ident("default") => {
                if self.eat("(")? {
                    self.ty()?;
                    self.expect(")")?;
                }
                Sk::Op("default", Vec::new())
            }
            Tok::Ident("new") => {
                let ty = self.type_name()?;
                if self.eat("[")? {
                    let len = self.expr(depth)?;
                    self.expect("]")?;
                    Sk::Op("new[]", vec![Sk::Name(ty), len])
                } else {
                    self.expect("(")?;
                    made(ty, self.list(")", depth)?)
                }
            }
            Tok::Ident(name) => Sk::Name(name.to_owned()),
            other => {
                let what = format!("an expression expected, not {}", describe(&other));
                return Err(ReparseError::Syntax { line, what });
            }
        })
    }
}

/// `e` without the casts around it.
fn under_casts(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// Whether `e` is a constant with no bit set, under any reading of it.
fn is_zero(e: &Expr) -> bool {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } | Expr::Truth(inner) = e {
        e = &**inner;
    }
    matches!(e, Expr::Const(c) if c.bits == 0)
}

/// The first argument of the call `e` is under any reading or test of it: the receiver the printer names.
fn receiver(e: &Expr) -> Option<&Expr> {
    let mut e = e;
    loop {
        match e {
            Expr::Cast { e: inner, .. }
            | Expr::Sugar { e: inner, .. }
            | Expr::Truth(inner)
            | Expr::Unary { e: inner, .. } => e = &**inner,
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } if is_zero(r) => e = &**l,
            Expr::Call { args, .. } => return args.first(),
            _ => return None,
        }
    }
}

fn assigned_receiver(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Assign { value, .. } => receiver(value),
        _ => None,
    }
}

/// The last part of the name a call goes by.
fn last_name(callee: &Callee) -> &str {
    let name = match callee {
        Callee::Local { name, .. }
        | Callee::Import { name, .. }
        | Callee::Native { name, .. }
        | Callee::Intrinsic { name, .. } => name.as_str(),
        Callee::Builtin { path, .. } => path.as_str(),
        Callee::Virtual { method, .. } => method.as_str(),
    };
    name.rsplit('.').next().unwrap_or(name)
}

/// `++` or `--` where `value` is `target` one up or down in 32 bits, which the printer writes as a step.
fn step_of(target: Var, value: &Expr) -> Option<&'static str> {
    let Expr::Binary {
        op,
        at: Kind::I32,
        l,
        r,
    } = value
    else {
        return None;
    };
    let mut stepped = &**l;
    while let Expr::Cast {
        to: Scalar::I32,
        e: inner,
    } = stepped
    {
        stepped = &**inner;
    }
    let one = matches!(&**r, Expr::Const(c) if c.bits == 1);
    let itself = matches!(stepped, Expr::Var(var) if *var == target);
    match (op, one && itself) {
        (BinOp::Add, true) => Some("++"),
        (BinOp::Sub, true) => Some("--"),
        _ => None,
    }
}

/// A constant as its kind has the printer spell it.
fn constant(c: &Const) -> Sk {
    let bits = match c.kind {
        Kind::Bool => u64::from(c.bits != 0),
        Kind::I8 => c.bits as i8 as i64 as u64,
        Kind::U8 => u64::from(c.bits as u8),
        Kind::I16 => c.bits as i16 as i64 as u64,
        Kind::U16 => u64::from(c.bits as u16),
        Kind::I32 => c.bits as i32 as i64 as u64,
        Kind::U32 | Kind::Hash | Kind::F32 => u64::from(c.bits as u32),
        _ => c.bits,
    };
    let lit = match c.kind {
        Kind::F32 => Lit::F32,
        Kind::F64 => Lit::F64,
        _ => Lit::Int,
    };
    Sk::Num(bits, lit)
}

/// A function as the readable form states it, with no names list.
// ⚠ Each rule here is the printer's: a form `print.rs` writes another way must change here too,
// and until it does the two differ, which is what the check is for.
struct Held<'a> {
    head: &'a Head,
    names: Names,
    /// The statements and operands the function may still come to, of `limit`.
    left: Cell<usize>,
    limit: usize,
}

impl Held<'_> {
    /// Takes one statement or operand of what the function may come to.
    fn spend(&self) -> Result<(), ReparseError> {
        match self.left.get().checked_sub(1) {
            Some(left) => {
                self.left.set(left);
                Ok(())
            }
            None => Err(ReparseError::TooLarge { limit: self.limit }),
        }
    }

    /// Whether a variable is declared a 32-bit integer, the only kind the printer writes a step of.
    fn is_int(&self, var: Var) -> bool {
        let kind = self.head.vars.get(&var).map(|info| &info.kind);
        matches!(kind, Some(Kind::I32 | Kind::U32))
    }

    fn var(&self, var: Var) -> String {
        if let Some(name) = self.head.vars.get(&var).and_then(|i| i.name.as_deref()) {
            return name.to_owned();
        }
        match var {
            Var::Global {
                program,
                offset,
                statics,
            } => {
                let space = if statics { "Static" } else { "Global" };
                self.owned(
                    program,
                    format!("{space}_{offset:X}"),
                    format!("{space}_{program:08X}_{offset:X}"),
                )
            }
            other => other.to_string(),
        }
    }

    /// What belongs to `program`: `own` in the function's program, `other` in another.
    fn owned(&self, program: u32, own: String, other: String) -> String {
        match program == self.head.program {
            true => own,
            false => other,
        }
    }

    fn function(&self, program: u32, at: u32) -> String {
        self.owned(
            program,
            format!("func_{at:06X}"),
            format!("program_{program:08X}.func_{at:06X}"),
        )
    }

    fn list(&self, items: &[Expr], depth: u32) -> Result<Vec<Sk>, ReparseError> {
        items.iter().map(|e| self.sk(e, depth)).collect()
    }

    fn sk(&self, e: &Expr, depth: u32) -> Result<Sk, ReparseError> {
        self.spend()?;
        let d = deeper(depth, 0)?;
        Ok(match e {
            Expr::Const(c) => constant(c),
            Expr::Sugar { kind, e } => self.sugared(*kind, e, d)?,
            Expr::Var(var) => Sk::Name(self.var(*var)),
            Expr::GuidEmpty => Sk::Name("Guid.Empty".to_owned()),
            Expr::Cell { base, index } => index_of(self.sk(base, d)?, num(*index)),
            Expr::Cells(items) => Sk::Op("{}", self.list(items, d)?),
            // A constant under a cast is written as the constant the cast reads.
            Expr::Cast { to, e: inner } => match (under_casts(inner), self.sk(inner, d)?) {
                (Expr::Const(_), Sk::Num(bits, _)) => {
                    let lit = match to {
                        Scalar::F32 => Lit::F32,
                        Scalar::F64 => Lit::F64,
                        _ => Lit::Int,
                    };
                    Sk::Num(cast(*to, bits), lit)
                }
                (_, other) => other,
            },
            Expr::Truth(e) => truth(self.sk(e, d)?),
            Expr::Unary { op: UnOp::Not, e } => not(self.sk(e, d)?),
            Expr::Unary { op: UnOp::Neg, e } => neg(self.sk(e, d)?),
            Expr::Unary {
                op: UnOp::BitNot,
                e,
            } => Sk::Op("~", vec![self.sk(e, d)?]),
            Expr::Binary {
                op: op @ (BinOp::LogicalAnd | BinOp::LogicalOr),
                ..
            } => self.chain(*op, e, d)?,
            Expr::Binary { .. } => {
                // A long sum is a chain down its left side: walked, not recursed into.
                let mut spine = Vec::new();
                let mut left = e;
                while let Expr::Binary { op, l, r, .. } = left {
                    if matches!(op, BinOp::LogicalAnd | BinOp::LogicalOr) {
                        break;
                    }
                    spine.push((*op, &**r));
                    left = &**l;
                }
                let mut all = self.sk(left, d)?;
                for (op, r) in spine.into_iter().rev() {
                    all = binary(op.symbol(), all, self.sk(r, d)?);
                }
                all
            }
            Expr::Select { cond, then, els } => Sk::Op(
                "?:",
                vec![
                    truth(self.sk(cond, d)?),
                    self.sk(then, d)?,
                    self.sk(els, d)?,
                ],
            ),
            Expr::Call { callee, args, .. } => self.call(callee, args, d)?,
            Expr::Load { addr, offset, .. } => self.place(addr, *offset, false, d)?,
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if bytes.is_multiple_of(8) => self.place(addr, *offset, false, d)?,
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => call(
                "load_bytes",
                vec![self.sk(addr, d)?, num(*offset), num(*bytes)],
            ),
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } => call(
                "extract_bytes",
                vec![self.sk(value, d)?, num(*offset), num(*bytes)],
            ),
            Expr::ByteOffset { addr, offset } => match &**offset {
                Expr::Const(c) if c.bits == 0 => self.sk(addr, d)?,
                Expr::Const(c) if c.bits <= u64::from(u32::MAX) => {
                    Sk::Op("&", vec![self.place(addr, c.bits as u32, false, d)?])
                }
                _ => call("byte_offset", vec![self.sk(addr, d)?, self.sk(offset, d)?]),
            },
            Expr::AddrOf { var, cell: 0 } => Sk::Op("&", vec![Sk::Name(self.var(*var))]),
            Expr::AddrOf { var, cell } => {
                Sk::Op("&", vec![index_of(Sk::Name(self.var(*var)), num(*cell))])
            }
            Expr::ZeroSlots(_) => Sk::Op("default", Vec::new()),
            Expr::Vector3(parts) => made("Vector3".to_owned(), self.list(&parts[..], d)?),
            Expr::NewObject(ty) => made(ty.name.clone(), Vec::new()),
            Expr::NewArray { ty, len, .. } => {
                Sk::Op("new[]", vec![Sk::Name(ty.name.clone()), self.sk(len, d)?])
            }
            Expr::FunctionRef { program, at } => {
                Sk::Op("&", vec![Sk::Name(self.function(*program, *at))])
            }
            Expr::BuiltinFunctionRef { ty, method } => {
                call("BuiltinFunction", vec![num(*ty), num(*method)])
            }
            Expr::VirtualFunctionRef { receiver, ty, slot } => call(
                "VirtualFunction",
                vec![self.sk(receiver, d)?, Sk::Name(ty.name.clone()), num(*slot)],
            ),
            Expr::StringConstant { program, offset } => Sk::Name(self.owned(
                *program,
                format!("String_{offset:X}"),
                format!("String_{program:08X}_{offset:X}"),
            )),
            Expr::BuiltinGlobal(n) => call("BuiltinGlobal", vec![num(*n)]),
        })
    }

    /// A chain of one of `&&` and `||` as one list, however the chain nests.
    fn chain(&self, op: BinOp, e: &Expr, depth: u32) -> Result<Sk, ReparseError> {
        let mut operands = Vec::new();
        let mut open = vec![e];
        while let Some(e) = open.pop() {
            match e {
                Expr::Binary {
                    op: under, l, r, ..
                } if *under == op => {
                    open.push(&**r);
                    open.push(&**l);
                }
                operand => operands.push(operand),
            }
        }
        let mut items = Vec::new();
        for operand in operands {
            match truth(self.sk(operand, depth)?) {
                Sk::Op(mark, inner) if mark == op.symbol() => items.extend(inner),
                other => items.push(other),
            }
        }
        Ok(Sk::Op(op.symbol(), items))
    }

    /// `e` as the source construct `kind` reads it, where `e` has the shape that construct takes.
    fn sugared(&self, kind: ExprSugar, e: &Expr, d: u32) -> Result<Sk, ReparseError> {
        Ok(match (kind, e) {
            (ExprSugar::Method, Expr::Call { callee, args, .. }) if !args.is_empty() => called(
                member(self.sk(&args[0], d)?, last_name(callee)),
                self.list(&args[1..], d)?,
            ),
            (ExprSugar::Property, Expr::Call { callee, args, .. }) if args.len() == 1 => {
                member(self.sk(&args[0], d)?, last_name(callee))
            }
            (ExprSugar::Field { .. }, Expr::Load { addr, offset, .. }) => {
                Sk::Op(".f", vec![self.sk(addr, d)?, num(*offset)])
            }
            (ExprSugar::Lambda, Expr::FunctionRef { program, at }) => {
                Sk::Name(self.function(*program, *at))
            }
            (ExprSugar::Coalesce, Expr::Select { then, els, .. }) => {
                Sk::Op("??", vec![self.sk(then, d)?, self.sk(els, d)?])
            }
            (ExprSugar::Concat, Expr::Call { args, .. }) if !args.is_empty() => {
                let mut all = self.sk(&args[0], d)?;
                for arg in &args[1..] {
                    all = binary("+", all, self.sk(arg, d)?);
                }
                all
            }
            (ExprSugar::Out | ExprSugar::Ref, Expr::AddrOf { var, cell: 0 }) => {
                Sk::Op("&", vec![Sk::Name(self.var(*var))])
            }
            (ExprSugar::Tuple, Expr::Cells(items)) => tuple(self.list(items, d)?),
            _ => self.sk(e, d)?,
        })
    }

    fn call(&self, callee: &Callee, args: &[Expr], d: u32) -> Result<Sk, ReparseError> {
        if let Callee::Intrinsic { name, .. } = callee {
            if let Some(spelled) = self.intrinsic(name, args, d)? {
                return Ok(spelled);
            }
        }
        let name = unquoted(callee_name(callee, &self.names));
        let passed = self.list(args, d)?;
        let form = match (callee, args) {
            // A receiver whose own type is the class is written in front of the method.
            (Callee::Virtual { method, .. }, [_, ..]) => Some(Form::Method(method.clone())),
            // A type's own equality over two values of several cells is written with its sign.
            (Callee::Builtin { path, .. }, [_, _]) => match path.rsplit('.').next() {
                Some("op_Equality") => Some(Form::Sign {
                    mark: "==",
                    not: false,
                }),
                Some("op_Inequality") => Some(Form::Sign {
                    mark: "!=",
                    not: false,
                }),
                _ => None,
            },
            _ => None,
        };
        Ok(match form {
            Some(form) => Sk::Call {
                name,
                form,
                args: passed,
            },
            None => called(Sk::Name(name), passed),
        })
    }

    fn element(&self, array: &Expr, index: &Expr, d: u32) -> Result<Sk, ReparseError> {
        Ok(index_of(self.sk(array, d)?, self.sk(index, d)?))
    }

    /// An intrinsic the printer writes as an operator; `None` for one that stays a call.
    fn intrinsic(&self, name: &str, args: &[Expr], d: u32) -> Result<Option<Sk>, ReparseError> {
        let element = |name: &str| {
            name == "array_get_value"
                || (name.strip_prefix("array_get_")).is_some_and(|t| Scalar::from_name(t).is_some())
        };
        Ok(Some(match args {
            [target, passed @ ..] if name == "invoke_delegate" => {
                called(self.sk(target, d)?, self.list(passed, d)?)
            }
            [array] if name == "array_length" => member(self.sk(array, d)?, "Length"),
            [value] => match name
                .strip_prefix("try_cast<")
                .and_then(|t| t.strip_suffix('>'))
            {
                Some(ty) => Sk::Op("as", vec![self.sk(value, d)?, Sk::Name(ty.to_owned())]),
                None => return Ok(None),
            },
            [array, index] if name == "array_element_address" => {
                Sk::Op("&", vec![self.element(array, index, d)?])
            }
            [array, index] if element(name) => self.element(array, index, d)?,
            _ => return Ok(None),
        }))
    }

    /// An intrinsic call the printer writes as an assignment to an element.
    fn assignment(&self, e: &Expr, d: u32) -> Result<Option<Sk>, ReparseError> {
        let Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        } = e
        else {
            return Ok(None);
        };
        let ([array, index, cells @ ..], Some(kind)) =
            (args.as_slice(), name.strip_prefix("array_set_"))
        else {
            return Ok(None);
        };
        let value = match (kind, cells) {
            ("ref" | "u64" | "slots", [value]) => self.sk(value, d)?,
            ("u32", [value]) => self.stored(Scalar::U32, value, d)?,
            ("u16", [value]) => self.stored(Scalar::U16, value, d)?,
            ("u8", [value]) => self.stored(Scalar::U8, value, d)?,
            ("slots", [_, _, ..]) => Sk::Op("{}", self.list(cells, d)?),
            _ => return Ok(None),
        };
        Ok(Some(assign(self.element(array, index, d)?, value)))
    }

    /// `value` as a store of `to` writes it: a constant is the bits the store leaves.
    fn stored(&self, to: Scalar, value: &Expr, d: u32) -> Result<Sk, ReparseError> {
        Ok(match (under_casts(value), self.sk(value, d)?) {
            (Expr::Const(_), Sk::Num(bits, lit)) => Sk::Num(cast(to, bits), lit),
            (_, other) => other,
        })
    }

    /// The memory at byte `offset` from `addr`, as the printer's fields of what `addr` points at.
    fn place(&self, addr: &Expr, offset: u32, partial: bool, d: u32) -> Result<Sk, ReparseError> {
        let mut fields = vec![u64::from(offset)];
        let mut base = addr;
        while let Expr::ByteOffset { addr, offset } = base {
            match &**offset {
                Expr::Const(c) if c.bits <= u64::from(u32::MAX) => fields.push(c.bits),
                _ => break,
            }
            base = &**addr;
        }
        let embedded = fields.len() > 1;
        let (mut place, variable) = match base {
            Expr::AddrOf { var, cell: 0 } => (Sk::Name(self.var(*var)), true),
            Expr::AddrOf { var, cell } => (index_of(Sk::Name(self.var(*var)), num(*cell)), true),
            pointer if !embedded && offset == 0 => {
                return Ok(Sk::Op("*", vec![self.sk(pointer, d)?]));
            }
            pointer => (self.sk(pointer, d)?, false),
        };
        // `->` goes through a pointer and `.` stays in place.
        let mut through = !variable;
        for (n, field) in fields.iter().enumerate().rev() {
            let unsaid = *field == 0 && n == 0 && (embedded || (variable && !partial));
            if !unsaid {
                let mark = if std::mem::take(&mut through) {
                    "->"
                } else {
                    ".f"
                };
                place = Sk::Op(mark, vec![place, Sk::Num(*field, Lit::Int)]);
            }
        }
        Ok(place)
    }

    /// A simple statement as the printer writes it.
    fn simple(&self, stmt: &Stmt, d: u32) -> Result<Sk, ReparseError> {
        Ok(match stmt {
            Stmt::Assign { target, value } => match step_of(*target, value) {
                Some(step) if self.is_int(*target) => {
                    Sk::Op(step, vec![Sk::Name(self.var(*target))])
                }
                _ => assign(Sk::Name(self.var(*target)), self.sk(value, d)?),
            },
            Stmt::AssignMany { targets, values } => {
                let targets = targets.iter().map(|v| Sk::Name(self.var(*v)));
                assign(tuple(targets.collect()), tuple(self.list(values, d)?))
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                let narrow = match kind {
                    StoreKind::U8 => Some(Scalar::U8),
                    StoreKind::U16 => Some(Scalar::U16),
                    StoreKind::I32 => Some(Scalar::I32),
                    StoreKind::U64 | StoreKind::Ref => None,
                };
                let place = self.place(addr, *offset, narrow.is_some(), d)?;
                let value = match narrow {
                    Some(to) => self.stored(to, value, d)?,
                    None => self.sk(value, d)?,
                };
                assign(place, value)
            }
            // Whole cells of a value as wide as the store are the field assigned.
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } if bytes.is_multiple_of(8) && cells_of(value, self.head) == Some(bytes / 8) => {
                assign(self.place(addr, *offset, true, d)?, self.sk(value, d)?)
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } => call(
                "store_bytes",
                vec![
                    self.sk(addr, d)?,
                    num(*offset),
                    num(*bytes),
                    self.sk(value, d)?,
                ],
            ),
            Stmt::StoreF32Slots {
                addr,
                offset,
                value,
            } => call(
                "store_f32_slots",
                vec![self.sk(addr, d)?, num(*offset), self.sk(value, d)?],
            ),
            Stmt::ZeroBytes { addr, bytes } => {
                call("zero_bytes", vec![self.sk(addr, d)?, num(*bytes)])
            }
            Stmt::StepI32 { addr, offset, down } => {
                let mark = if *down { "--" } else { "++" };
                Sk::Op(mark, vec![self.place(addr, *offset, true, d)?])
            }
            Stmt::Eval(e) => match self.assignment(e, d)? {
                Some(assigned) => assigned,
                None => self.sk(e, d)?,
            },
        })
    }

    fn simples(&self, parts: &[Stmt], out: &mut Vec<Node>, d: u32) -> Result<(), ReparseError> {
        for part in parts {
            out.push(Node::held("", vec![self.simple(part, d)?], Vec::new()));
        }
        Ok(())
    }

    /// `x = new T(a, b)` for an object made by one statement and constructed by the next.
    fn constructed(
        &self,
        made_by: &tree::Stmt,
        next: Option<&tree::Stmt>,
        d: u32,
    ) -> Result<Option<Sk>, ReparseError> {
        let tree::Stmt::Simple(Stmt::Assign {
            target,
            value: Expr::NewObject(ty),
        }) = made_by
        else {
            return Ok(None);
        };
        let Some(tree::Stmt::Simple(Stmt::Eval(Expr::Call {
            callee: Callee::Builtin { path, .. },
            args,
            ..
        }))) = next
        else {
            return Ok(None);
        };
        let Some((receiver, passed)) = args.split_first() else {
            return Ok(None);
        };
        let own = path.strip_suffix(".Ctor") == Some(ty.name.as_str());
        let mut again = false;
        for arg in passed {
            arg.walk(&mut |e| {
                again |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if v == target);
            });
        }
        if !own || *receiver != Expr::Var(*target) || again {
            return Ok(None);
        }
        let object = made(ty.name.clone(), self.list(passed, d)?);
        Ok(Some(assign(Sk::Name(self.var(*target)), object)))
    }

    /// The statements of a block as the printer writes them.
    fn nodes(&self, stmts: &[tree::Stmt], depth: u32) -> Result<Vec<Node>, ReparseError> {
        let d = deeper(depth, 0)?;
        let mut out = Vec::new();
        let mut n = 0;
        while let Some(stmt) = stmts.get(n) {
            n += 1;
            if let Some(pair) = self.constructed(stmt, stmts.get(n), d)? {
                out.push(Node::held("", vec![pair], Vec::new()));
                n += 1;
                continue;
            }
            self.node(stmt, &mut out, d)?;
        }
        Ok(out)
    }

    fn node(&self, stmt: &tree::Stmt, out: &mut Vec<Node>, d: u32) -> Result<(), ReparseError> {
        self.spend()?;
        let bare = |word| Node::held(word, Vec::new(), Vec::new());
        let node = match stmt {
            tree::Stmt::Simple(s) => Node::held("", vec![self.simple(s, d)?], Vec::new()),
            tree::Stmt::If { cond, then, els } => {
                let test = vec![truth(self.sk(cond, d)?)];
                out.push(Node::held("if", test, vec![self.nodes(then, d)?]));
                // An `else` that is one `if` is written `else if`, however long the chain.
                let mut rest = els;
                loop {
                    match rest.as_slice() {
                        [] => return Ok(()),
                        [tree::Stmt::If { cond, then, els }] => {
                            let test = vec![truth(self.sk(cond, d)?)];
                            out.push(Node::held("else if", test, vec![self.nodes(then, d)?]));
                            rest = els;
                        }
                        other => break Node::held("else", Vec::new(), vec![self.nodes(other, d)?]),
                    }
                }
            }
            tree::Stmt::While { cond, body } => Node::held(
                "while",
                vec![truth(self.sk(cond, d)?)],
                vec![self.nodes(body, d)?],
            ),
            tree::Stmt::DoWhile { body, cond } => Node::held(
                "do",
                vec![truth(self.sk(cond, d)?)],
                vec![self.nodes(body, d)?],
            ),
            tree::Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let part = |s: &Option<Box<Stmt>>| match s {
                    Some(s) => self.simple(s, d),
                    None => Ok(none()),
                };
                let cond = match cond {
                    Some(cond) => truth(self.sk(cond, d)?),
                    None => none(),
                };
                let head = vec![part(init)?, cond, part(step)?];
                Node::held("for", head, vec![self.nodes(body, d)?])
            }
            tree::Stmt::Switch { value, cases } => {
                let mut all: Vec<Node> = Vec::new();
                for case in cases {
                    let labels = case.labels.iter().map(|label| match label {
                        Some(value) => Sk::Num(*value as u64, Lit::Int),
                        None => Sk::Name("default".to_owned()),
                    });
                    let labels: Vec<Sk> = labels.collect();
                    let body = self.nodes(&case.body, d)?;
                    // A case that writes no statement runs into the next: one case in the text.
                    let joins = all.last().is_some_and(runs_on);
                    match all.last_mut() {
                        Some(last) if joins => {
                            last.head.extend(labels);
                            last.blocks = vec![body];
                        }
                        _ => all.push(Node::held("case", labels, vec![body])),
                    }
                }
                Node::held("switch", vec![self.sk(value, d)?], vec![all])
            }
            tree::Stmt::Break => bare("break"),
            tree::Stmt::Continue => bare("continue"),
            tree::Stmt::Unreachable => bare("unreachable"),
            tree::Stmt::Return(None) => bare("return"),
            tree::Stmt::Return(Some(value)) => {
                Node::held("return", vec![self.sk(value, d)?], Vec::new())
            }
            tree::Stmt::Goto(label) => {
                Node::held("goto", vec![Sk::Name(label.to_string())], Vec::new())
            }
            tree::Stmt::Label(label) => {
                Node::held("label", vec![Sk::Name(label.to_string())], Vec::new())
            }
            tree::Stmt::Sugar(sugar) => return self.sugar(sugar, out, d),
        };
        out.push(node);
        Ok(())
    }

    /// The statements a source construct stands for, as the printer writes them where it has no form for the construct.
    fn lowered(
        &self,
        sugar: &tree::Sugar,
        out: &mut Vec<Node>,
        d: u32,
    ) -> Result<(), ReparseError> {
        out.extend(self.nodes(&sugar.clone().desugar(), d)?);
        Ok(())
    }

    /// A source construct: its keyword and its blocks where its parts say what to write.
    fn sugar(&self, sugar: &tree::Sugar, out: &mut Vec<Node>, d: u32) -> Result<(), ReparseError> {
        match sugar {
            tree::Sugar::Hidden(_) => {}
            tree::Sugar::Group { parts, .. } => self.simples(parts, out, d)?,
            tree::Sugar::Await {
                before,
                ready,
                after,
                ..
            } => {
                self.simples(before, out, d)?;
                let awaited = self.sk(receiver(ready).unwrap_or(ready), d)?;
                out.push(Node::held("await", vec![awaited], Vec::new()));
                self.simples(after, out, d)?;
            }
            tree::Sugar::Yield { value, .. } => {
                self.simples(value, out, d)?;
                out.push(Node::held("yield", Vec::new(), Vec::new()));
            }
            tree::Sugar::Foreach {
                setup,
                cond,
                item,
                body,
                after,
                ..
            } => {
                let (shown, source) = match setup.split_last() {
                    Some((taken, rest)) if assigned_receiver(taken).is_some() => {
                        (rest, assigned_receiver(taken))
                    }
                    _ => (setup.as_slice(), receiver(cond)),
                };
                let (Stmt::Assign { target, .. }, Some(source)) = (&**item, source) else {
                    return self.lowered(sugar, out, d);
                };
                self.simples(shown, out, d)?;
                let head = vec![Sk::Name(self.var(*target)), self.sk(source, d)?];
                out.push(Node::held("foreach", head, vec![self.nodes(body, d)?]));
                out.extend(self.nodes(after, d)?);
            }
            tree::Sugar::Guarded {
                kind, enter, body, ..
            } => {
                let (taken, shown) = match enter.split_last() {
                    Some((taken, rest)) => (Some(taken), rest),
                    None => (None, enter.as_slice()),
                };
                self.simples(shown, out, d)?;
                let held = match (kind, taken) {
                    (tree::Guard::Lock, Some(Stmt::Eval(Expr::Call { args, .. })))
                        if !args.is_empty() =>
                    {
                        vec![self.sk(&args[0], d)?]
                    }
                    (_, Some(taken)) => vec![self.simple(taken, d)?],
                    (_, None) => Vec::new(),
                };
                let word = match kind {
                    tree::Guard::Using => "using",
                    tree::Guard::Lock => "lock",
                };
                out.push(Node::held(word, held, vec![self.nodes(body, d)?]));
            }
            tree::Sugar::Try {
                body,
                catches,
                finally,
            } => {
                out.push(Node::held("try", Vec::new(), vec![self.nodes(body, d)?]));
                for catch in catches {
                    let mut head = Vec::new();
                    if let Some(ty) = catch.ty {
                        head.push(Sk::Name(ty.to_string()));
                        head.extend(catch.var.map(|var| Sk::Name(self.var(var))));
                    }
                    out.push(Node::held("catch", head, vec![self.nodes(&catch.body, d)?]));
                }
                if !finally.is_empty() {
                    let block = vec![self.nodes(finally, d)?];
                    out.push(Node::held("finally", Vec::new(), block));
                }
            }
            tree::Sugar::Throw(raise) => match &**raise {
                Stmt::Eval(Expr::Call { args, .. }) if args.len() == 1 => {
                    out.push(Node::held("throw", vec![self.sk(&args[0], d)?], Vec::new()));
                }
                other => {
                    out.push(Node::held("", vec![self.simple(other, d)?], Vec::new()));
                    out.push(Node::held("throw", Vec::new(), Vec::new()));
                }
            },
            tree::Sugar::SwitchOn { .. }
            | tree::Sugar::Event { .. }
            | tree::Sugar::Cached { .. } => self.lowered(sugar, out, d)?,
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "reparse_tests.rs"]
mod tests;
