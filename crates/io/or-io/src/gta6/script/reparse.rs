//! Reading the printer's own output back: the check that the text says what the tree holds, which
//! running a function before and after the passes cannot make.
//!
//! The text and the function each come to one form, statements in their nesting over operator
//! trees with every reading taken off: a cast, a truth test and a pair of parentheses state no
//! operand, so `(int)x != 0` and `x` are one.
//! ⚠ What that form leaves out is not checked: a cast dropped where it counted reads the same.

use std::cell::Cell;
use std::fmt;

use super::decode::TYPE_TOKEN;
use super::ir::{
    BinOp, Callee, Const, Expr, ExprSugar, Head, Kind, Scalar, Stmt, StoreKind, UnOp, Var,
};
use super::names::Names;
use super::passes::async_await;
use super::print::{callee_name, cells_of, reads_through};
use super::tree;
use super::{Level, Options, Pass};

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
        /// The innermost two forms where the text and the function part, by their marks alone: what a count of differences is kept by.
        form: String,
    },
    #[error("more than {limit} statements and operands: not read back")]
    TooLarge { limit: usize },
    /// The text or the function holds a source form the reader does not hold against the other yet: counted by `form`, and compared no further.
    #[error("line {line}: {form} is not read back yet")]
    Unread { line: u32, form: String },
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
                sharp: None,
                this: None,
            };
            compare(&held.nodes(&f.body, 0)?, &body)
        }
    }
}

/// Checks that `text`, written with no names list as the source level writes `f` by the passes `options` has on, says what `f` holds; with `receiver`, `this` and `base` are its first parameter. A form not read back yet is `Unread`.
pub fn agrees_source(
    f: &tree::Function,
    text: &str,
    options: Options,
    receiver: bool,
) -> Result<(), ReparseError> {
    agrees_source_within(f, text, options, receiver, MAX_NODES)
}

/// [`agrees_source`], for a function and a text of no more than `limit` statements and operands each.
pub fn agrees_source_within(
    f: &tree::Function,
    text: &str,
    options: Options,
    receiver: bool,
    limit: usize,
) -> Result<(), ReparseError> {
    let held = Held {
        head: &f.head,
        names: Names::default(),
        left: Cell::new(limit),
        limit,
        sharp: Some(options),
        this: receiver.then_some(Var::Param(0)),
    };
    let this = receiver.then(|| held.var(Var::Param(0)));
    let mut read_back = None;
    read_in(text, limit, this, &mut |name, body| {
        if name == f.head.name && read_back.is_none() {
            read_back = Some(body);
        }
    })?;
    match (read_back, &f.head.incomplete) {
        (None, Some(_)) => Ok(()),
        (None, None) => Err(missing("no such function", &f.head.name)),
        (Some(_), Some(_)) => Err(missing(&f.head.name, "a function with no body")),
        (Some(body), None) => compare(&held.nodes(&f.body, 0)?, &body),
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
    /// `value.f_at` for bytes taken out of the value the first operand is, or out of its item `item` where it is a group; with `whole`, the value alone.
    Field {
        item: Option<usize>,
        at: u64,
        whole: bool,
    },
    /// `new T(rest)` for a constructor run on an object the call makes, the first operand.
    Made(String),
}

/// A test that a cell is above zero as an unsigned number, which true C# writes `x != null` of a reference: the text may say either.
const ABOVE_ZERO: &str = "above zero";

/// The negation of [`ABOVE_ZERO`]: `x <= 0`, or `x == null` of a reference.
const AT_MOST_ZERO: &str = "at most zero";

/// `new T(a) { f_8 = b }`: the type, the constructor's arguments or none, then each field given its value.
const INIT: &str = "new {}";

/// The arguments of the constructor an object initialiser runs.
const CTOR: &str = "new ()";

/// What stands at the address of an element, which true C# writes `a[i]` where the access is as wide as an element and `(&a[i])->f_0` where it is not: the text may say either.
// ⚠ Held so only where no variable's kind the function states gives an element's width; where
// one does, the one form that width says is held.
const AT_ELEMENT: &str = "at an element";

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

/// A source form the reader does not hold against the function yet, by what it is.
fn unread(form: &str) -> Sk {
    Sk::Op(UNREAD, vec![Sk::Name(form.to_owned())])
}

/// The mark of [`unread`].
const UNREAD: &str = "unread";

/// A statement the function holds whose form the reader does not hold against the text yet.
fn unread_node(form: &str) -> Node {
    Node::held("", vec![unread(form)], Vec::new())
}

/// The form not read back yet that `items` hold, the first a walk meets.
// An explicit stack: a chain of operators nests as deep as the text is long.
fn unread_in<'s>(items: impl IntoIterator<Item = &'s Sk>) -> Option<String> {
    let mut open: Vec<&Sk> = items.into_iter().collect();
    while let Some(sk) = open.pop() {
        match sk {
            Sk::Op(UNREAD, form) => {
                let named = form.first().map(|form| form.to_string());
                return Some(named.unwrap_or_default());
            }
            Sk::Op(_, items) | Sk::Call { args: items, .. } => open.extend(items),
            Sk::Num(..) | Sk::Name(_) => {}
        }
    }
    None
}

/// Whether `e` is the exception true C# throws where control never comes.
fn unreachable(e: &Sk) -> bool {
    match e {
        Sk::Op("new", items) => matches!(
            items.as_slice(),
            [Sk::Name(name)] if name == "System.Diagnostics.UnreachableException"
        ),
        _ => false,
    }
}

/// The variable `this` stands for where `e` is it; else `e` as it is.
fn received(e: Sk) -> Result<Sk, Sk> {
    match e {
        Sk::Op("this", mut items) if items.len() == 1 => Ok(items.remove(0)),
        other => Err(other),
    }
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
        // The infinities the printer spells by name are the bits of a constant.
        Sk::Name(path) => match (path.as_str(), name) {
            ("float", "PositiveInfinity") => Sk::Num(u64::from(f32::INFINITY.to_bits()), Lit::F32),
            ("float", "NegativeInfinity") => {
                Sk::Num(u64::from(f32::NEG_INFINITY.to_bits()), Lit::F32)
            }
            ("double", "PositiveInfinity") => Sk::Num(f64::INFINITY.to_bits(), Lit::F64),
            ("double", "NegativeInfinity") => Sk::Num(f64::NEG_INFINITY.to_bits(), Lit::F64),
            _ => Sk::Name(format!("{path}.{name}")),
        },
        other => Sk::Op(".", vec![other, Sk::Name(name.to_owned())]),
    }
}

/// A call; a reading of bits that is written as one is its operand.
fn called(callee: Sk, mut args: Vec<Sk>) -> Sk {
    if let (Sk::Name(name), 1) = (&callee, args.len()) {
        let reading = name
            .strip_prefix("as_")
            .is_some_and(|ty| Scalar::from_name(ty).is_some());
        // `unchecked((int)0x89ABCDEF)` is true C#'s word for the bits of the cast it holds.
        if reading || name == "float_bits" || name == "unchecked" {
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
        Sk::Op(ABOVE_ZERO, items) => Sk::Op(AT_MOST_ZERO, items),
        Sk::Op(AT_MOST_ZERO, items) => Sk::Op(ABOVE_ZERO, items),
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
        (Form::Field { item, at, whole }, text) => {
            let of = match (item, args.first()) {
                (None, of) => of,
                (Some(n), Some(Sk::Op("{}", items))) => items.get(*n),
                (Some(_), _) => None,
            };
            let named = matches!(text, Sk::Op("()", items)
                if matches!(items.first(), Some(Sk::Name(callee)) if callee == name));
            match (named, of, text) {
                (true, ..) => by_name(text),
                (false, Some(of), Sk::Op(".f", pair)) => match pair.as_slice() {
                    [base, Sk::Num(offset, _)] => offset == at && same(of, base),
                    _ => false,
                },
                (false, Some(of), text) => *whole && same(of, text),
                (false, None, _) => false,
            }
        }
        (Form::Made(ty), Sk::Op("new", items)) => match (items.split_first(), args.split_first()) {
            (Some((Sk::Name(made), given)), Some((_, passed))) => made == ty && all(passed, given),
            _ => false,
        },
        (Form::Made(_), _) => by_name(text),
    }
}

fn same(held: &Sk, text: &Sk) -> bool {
    match (held, text) {
        (Sk::Call { name, form, args }, text) => same_call(name, form, args, text),
        (Sk::Op(ABOVE_ZERO, held), text) => {
            let [value] = held.as_slice() else {
                return false;
            };
            let compared = match text {
                Sk::Op(">", pair) => match pair.as_slice() {
                    [l, Sk::Num(0, _)] => Some(l),
                    _ => None,
                },
                _ => None,
            };
            match compared {
                Some(l) => same(value, l),
                None => same(value, text),
            }
        }
        // ⚠ The negation is made of the operand at each comparison and dropped: holding it beside
        // the operand would double what a chain of such tests one in another comes to.
        (Sk::Op(AT_MOST_ZERO, held), text) => {
            let [value] = held.as_slice() else {
                return false;
            };
            match text {
                Sk::Op("<=", pair) => match pair.as_slice() {
                    [l, Sk::Num(0, _)] => same(value, l),
                    _ => false,
                },
                text => same(&not(value.clone()), text),
            }
        }
        (Sk::Op(AT_ELEMENT, held), text) => {
            let element = |pair: &[Sk]| match (held.as_slice(), pair) {
                ([a, i], [x, y]) => same(a, x) && same(i, y),
                _ => false,
            };
            match text {
                Sk::Op("[]", pair) => element(pair),
                Sk::Op("->", through) => match through.as_slice() {
                    [Sk::Op("&", inner), Sk::Num(0, _)] => match inner.as_slice() {
                        [Sk::Op("[]", pair)] => element(pair),
                        _ => false,
                    },
                    _ => false,
                },
                _ => false,
            }
        }
        (Sk::Num(a, _), Sk::Num(b, _)) => same_bits(*a, *b),
        (Sk::Name(a), Sk::Name(b)) => a == b,
        (Sk::Op(a, xs), Sk::Op(b, ys)) => {
            a == b && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same(x, y))
        }
        _ => false,
    }
}

/// A function the text and the tree do not both hold, as the difference it is.
fn missing(text: &str, held: &str) -> ReparseError {
    ReparseError::Differs {
        line: 0,
        text: text.to_owned(),
        held: held.to_owned(),
        form: "a function".to_owned(),
    }
}

fn differs(line: u32, text: Option<&Node>, held: Option<&Node>) -> ReparseError {
    let shown = |node: Option<&Node>| node.map_or("nothing".to_owned(), Node::to_string);
    ReparseError::Differs {
        line,
        text: shown(text),
        held: shown(held),
        form: parted(text, held),
    }
}

/// The pairs of operands followed into, at most, to find where two statements part.
const PARTING: u32 = 64;

/// A form by its mark alone: an operator's, a helper's name, or what kind of operand it is.
// ⚠ A name of the program is no mark: the counts would part by every variable and callee.
fn mark(sk: &Sk) -> String {
    let helper = |name: &str| {
        let plain = name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        plain && !name.starts_with("func_") && !name.starts_with('_')
    };
    match sk {
        Sk::Num(..) => "a number".to_owned(),
        Sk::Name(_) => "a name".to_owned(),
        Sk::Op("()", items) => match items.first() {
            Some(Sk::Name(name)) if helper(name.as_str()) => format!("`{name}`"),
            _ => "a call".to_owned(),
        },
        Sk::Op(mark, _) => format!("`{mark}`"),
        Sk::Call { name, .. } if helper(name.as_str()) => format!("`{name}`"),
        Sk::Call { .. } => "a call".to_owned(),
    }
}

/// The innermost operands where `held` and `text` part, followed while both are the same operator over as many operands.
fn parting<'s>(held: &'s Sk, text: &'s Sk) -> (&'s Sk, &'s Sk) {
    let (mut h, mut t) = (held, text);
    for _ in 0..PARTING {
        let (Sk::Op(a, xs), Sk::Op(b, ys)) = (h, t) else {
            break;
        };
        if a != b || xs.len() != ys.len() {
            break;
        }
        match xs.iter().zip(ys).find(|(x, y)| !same(x, y)) {
            Some((x, y)) => (h, t) = (x, y),
            None => break,
        }
    }
    (h, t)
}

/// What two statements that differ part at: their words, or the innermost forms of their heads.
fn parted(text: Option<&Node>, held: Option<&Node>) -> String {
    let word = |node: Option<&Node>| match node.map(|node| node.word) {
        None => "nothing".to_owned(),
        Some("") => "a statement".to_owned(),
        Some(word) => format!("`{word}`"),
    };
    let (Some(t), Some(h)) = (text, held) else {
        return format!("{} against {}", word(text), word(held));
    };
    if t.word != h.word || t.head.len() != h.head.len() {
        return format!("{} against {}", word(text), word(held));
    }
    let pair = h.head.iter().zip(&t.head).find(|(h, t)| !same(h, t));
    match pair {
        Some((h, t)) => {
            let (h, t) = parting(h, t);
            format!("{} against {}", mark(t), mark(h))
        }
        None => format!("the blocks of {}", word(text)),
    }
}

/// Holds the statements of the text against those of the function, block by block.
// An explicit stack: a body nests as deep as its listing does.
fn compare(held: &[Node], text: &[Node]) -> Result<(), ReparseError> {
    let mut work = vec![(held, text)];
    while let Some((held, text)) = work.pop() {
        for n in 0..held.len().max(text.len()) {
            let (h, t) = (held.get(n), text.get(n));
            // ⚠ A form either side does not read yet says nothing of what follows it in its block.
            let forms = (h.into_iter().chain(t)).find_map(|node| unread_in(&node.head));
            if let Some(form) = forms {
                let line = t.or(text.last()).map_or(0, |t| t.line);
                return Err(ReparseError::Unread { line, form });
            }
            let (Some(h), Some(t)) = (h, t) else {
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
const PUNCT: [&str; 49] = [
    "<<=", ">>=", "??=", "->", "++", "--", "&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "??",
    "=>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "?.", "+", "-", "*", "/", "%", "<", ">",
    "&", "|", "^", "!", "~", "=", "?", ":", ";", ",", ".", "(", ")", "{", "}", "[", "]",
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
                // `0x3F800000 /* 1.0f */`: what a number of no proved kind also is, which states nothing more.
                Some(b'/') if bytes.get(self.pos + 1) == Some(&b'*') => {
                    let rest = &bytes[self.pos + 2..];
                    let Some(end) = rest.windows(2).position(|pair| pair == b"*/") else {
                        return Err(self.refused("a comment with no end"));
                    };
                    let lines = rest[..end].iter().filter(|b| **b == b'\n').count();
                    let lines = u32::try_from(lines).unwrap_or(u32::MAX);
                    self.line = self.line.saturating_add(lines);
                    self.pos += 2 + end + 2;
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
    /// The name of the variable `this` and `base` stand for, where the text is a method's.
    this: Option<String>,
}

/// Reads every function of `text`, each of no more than `limit` statements and operands, hands each to `each` and returns the simple statements read.
// One function's statements are held at a time.
fn read(
    text: &str,
    limit: usize,
    each: &mut dyn FnMut(String, Vec<Node>),
) -> Result<usize, ReparseError> {
    read_in(text, limit, None, each)
}

/// [`read`], with `this` and `base` read as the variable `this` names where it names one.
fn read_in(
    text: &str,
    limit: usize,
    this: Option<String>,
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
        this,
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
            } else if self.is("?") && self.nullable_ahead() {
                self.bump()?;
                text.push('?');
            } else {
                return Ok(text);
            }
        }
    }

    /// Whether a type and the name it declares start here: `int x =`, `var[2] pair;`, `void* a, b;`.
    fn declaration_ahead(&self) -> bool {
        self.declaration_ahead_of(&["=", ",", ";"])
    }

    /// Whether a type and the name it declares start here, and one of `after` follows the name.
    fn declaration_ahead_of(&self, after: &[&str]) -> bool {
        let mut ahead = self.clone();
        let typed = ahead.ty().is_ok() && matches!(ahead.tok, Tok::Ident(_));
        typed && ahead.bump().is_ok() && after.iter().any(|p| ahead.is(p))
    }

    /// Whether the `(` here opens a cast: `(int)`, `(var[2])`, and in true C# `(Cells2)x`, `(type_1)x`.
    // ⚠ A type no keyword names is a cast only before what starts an operand: `(x) - 1` is no cast.
    fn cast_ahead(&self) -> bool {
        let keyword = |word: &str| word == "var" || Scalar::ALL.iter().any(|s| s.keyword() == word);
        let mut ahead = self.clone();
        if ahead.bump().is_err() {
            return false;
        }
        let named = matches!(ahead.tok, Tok::Ident(word) if keyword(word));
        let typed = matches!(ahead.tok, Tok::Ident(_) | Tok::Str(_));
        if !(typed && ahead.ty().is_ok() && ahead.is(")")) {
            return false;
        }
        let operand = ahead.bump().is_ok()
            && match &ahead.tok {
                Tok::Ident(word) => !matches!(*word, "as" | "is" | "in"),
                Tok::Num(..) | Tok::Str(_) => true,
                Tok::Punct(p) => matches!(*p, "(" | "!" | "~"),
                Tok::End => false,
            };
        named || operand
    }

    /// Whether the `?` here makes the type before it nullable: a name or a `)` follows, as in `int? x` and `(int?)x`.
    fn nullable_ahead(&self) -> bool {
        let mut ahead = self.clone();
        ahead.bump().is_ok() && (matches!(ahead.tok, Tok::Ident(_)) || ahead.is(")"))
    }

    /// Whether the `<` here opens type arguments that a `(` follows, as in `box<type_1>(x)`.
    fn generic_call_ahead(&self) -> bool {
        let mut ahead = self.clone();
        ahead.generic(&mut String::new()).is_ok() && ahead.is("(")
    }

    /// One function: its head, then its body.
    fn function(&mut self) -> Result<(String, Vec<Node>), ReparseError> {
        self.modifiers()?;
        self.ty()?;
        let name = self.ident()?.to_owned();
        self.expect("(")?;
        while !self.is(")") {
            self.modifiers()?;
            self.ty()?;
            self.ident()?;
            if !self.eat(",")? {
                break;
            }
        }
        self.expect(")")?;
        Ok((name, self.block(0)?))
    }

    /// The words before a head or a parameter that say how it is declared, which state nothing the body does.
    fn modifiers(&mut self) -> Result<(), ReparseError> {
        const WORDS: [&str; 9] = [
            "static", "async", "virtual", "override", "public", "private", "unsafe", "ref", "out",
        ];
        while matches!(&self.tok, Tok::Ident(word) if WORDS.contains(word)) {
            self.bump()?;
        }
        Ok(())
    }

    /// The tokens up to the `}` that closes the `{` here, read and set aside: a source form this reader does not hold against the function.
    // Each token is one of what the function may come to: a text past the bound stops here.
    fn skip_braces(&mut self) -> Result<(), ReparseError> {
        self.expect("{")?;
        let mut open = 1u32;
        while open > 0 {
            self.spend()?;
            match self.bump()? {
                Tok::Punct("{") => open += 1,
                Tok::Punct("}") => open -= 1,
                Tok::End => return Err(self.unexpected("`}`")),
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether a lambda starts here: a name or a list of parameters in parentheses, then `=>`.
    fn lambda_ahead(&self) -> bool {
        let mut ahead = self.clone();
        if matches!(ahead.tok, Tok::Ident(_)) {
            return ahead.bump().is_ok() && ahead.is("=>");
        }
        if !ahead.is("(") {
            return false;
        }
        for _ in 0..MAX_TYPE_MARKS {
            if ahead.bump().is_err() {
                return false;
            }
            if ahead.is(")") {
                return ahead.bump().is_ok() && ahead.is("=>");
            }
            if matches!(ahead.tok, Tok::Punct("(" | "{" | "}" | ";") | Tok::End) {
                return false;
            }
        }
        false
    }

    /// A lambda, read and set aside: its parameters, then the expression or the block it is.
    fn lambda(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        while !self.is("=>") {
            if self.tok == Tok::End {
                return Err(self.unexpected("`=>`"));
            }
            self.bump()?;
        }
        self.bump()?;
        match self.is("{") {
            true => self.skip_braces()?,
            false => {
                self.expr(depth)?;
            }
        }
        Ok(unread("a lambda written where it is bound"))
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
            // True C# says a place control never reaches by the exception made for it.
            "throw" => match self.worded("throw", line, depth)? {
                node if node.head.len() == 1 && unreachable(&node.head[0]) => Node {
                    word: "unreachable",
                    head: Vec::new(),
                    ..node
                },
                node => node,
            },
            "goto" => {
                self.bump()?;
                // `goto case 3;` and `goto default;`: the case a switch's case runs on into.
                if self.word("case") || self.word("default") {
                    let head = vec![self.goto_case()?];
                    self.expect(";")?;
                    out.push(Node {
                        word: "goto case",
                        head,
                        blocks: Vec::new(),
                        line,
                    });
                    return Ok(());
                }
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
                let head = match self.is(";") {
                    true => Vec::new(),
                    false => vec![self.expr(depth)?],
                };
                self.expect(";")?;
                Node {
                    word: "yield",
                    head,
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

    /// The fields an object initialiser stores, `{ f_8 = b, f_10 = c }`, each as the field by its offset given its value; one it does not hold is a form not read back.
    // Read ahead, so that a form it does not hold is set aside whole: each initialiser inside one
    // is read once whether or not the one round it reads.
    fn initialiser(&mut self, depth: u32) -> Result<Vec<Sk>, ReparseError> {
        let mut ahead = self.clone();
        match ahead.fields(depth) {
            Ok(fields) => {
                *self = ahead;
                Ok(fields)
            }
            Err(ReparseError::TooLarge { limit }) => Err(ReparseError::TooLarge { limit }),
            Err(_) => {
                self.skip_braces()?;
                Ok(vec![unread("an object written with its fields")])
            }
        }
    }

    /// `{ name = value, .. }`.
    fn fields(&mut self, depth: u32) -> Result<Vec<Sk>, ReparseError> {
        let d = deeper(depth, self.line)?;
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.is("}") {
            self.spend()?;
            let name = self.ident()?;
            let place = match field(name) {
                Some(offset) => Sk::Num(offset, Lit::Int),
                None => Sk::Name(name.to_owned()),
            };
            self.expect("=")?;
            fields.push(assign(place, self.expr(d)?));
            if !self.eat(",")? {
                break;
            }
        }
        self.expect("}")?;
        Ok(fields)
    }

    /// The label a `goto` names after `case` or as `default`: a number, or a label the reader does not hold.
    fn goto_case(&mut self) -> Result<Sk, ReparseError> {
        if self.word("default") {
            self.bump()?;
            return Ok(Sk::Name("default".to_owned()));
        }
        self.expect_word("case")?;
        let negative = self.eat("-")?;
        let number = match &self.tok {
            Tok::Num(bits, Lit::Int) => Some(*bits),
            _ => None,
        };
        match number {
            Some(bits) => {
                self.bump()?;
                let bits = if negative { bits.wrapping_neg() } else { bits };
                Ok(Sk::Num(bits, Lit::Int))
            }
            // `goto case Hash("..")`: a label by the text a list gives for it.
            None => {
                while !self.is(";") && self.tok != Tok::End {
                    self.bump()?;
                }
                Ok(unread("goto case"))
            }
        }
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
        if let Some(mark) = self.compound() {
            // `place op= value` works on the place it writes: `place = place op value`.
            self.bump()?;
            let value = self.expr(depth)?;
            return Ok(Some(assign(target.clone(), binary(mark, target, value))));
        }
        Ok(Some(match (self.eat("=")?, target) {
            // `_ = x`: a value given to the discard is the value evaluated for its effect.
            (true, Sk::Name(discard)) if discard == "_" => self.expr(depth)?,
            (true, target) => assign(target, self.expr(depth)?),
            (false, target) => target,
        }))
    }

    /// The operator of the compound assignment here: `+` for `+=`.
    fn compound(&self) -> Option<&'static str> {
        let Tok::Punct(p) = &self.tok else {
            return None;
        };
        Some(match *p {
            "+=" => "+",
            "-=" => "-",
            "*=" => "*",
            "/=" => "/",
            "%=" => "%",
            "&=" => "&",
            "|=" => "|",
            "^=" => "^",
            "<<=" => "<<",
            ">>=" => ">>",
            _ => return None,
        })
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
        // `cache ??= made`: a delegate made once and kept.
        if self.eat("??=")? {
            self.expr(d)?;
            return Ok(unread("??="));
        }
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
                None if self.word("as") || self.word("is") => 8,
                None => return Ok(left),
            };
            if strength < min {
                return Ok(left);
            }
            let is = self.word("is");
            self.bump()?;
            left = match op {
                // No deeper: the strength rises with each operand, so this ends by itself.
                Some(op) => binary(op.symbol(), left, self.binary(strength + 1, depth)?),
                // `x is T` and `x is T y`: a test of what an object is.
                None if is => {
                    self.ty()?;
                    if matches!(self.tok, Tok::Ident(_)) {
                        self.bump()?;
                    }
                    unread("is")
                }
                None => Sk::Op("as", vec![left, Sk::Name(self.ty()?)]),
            };
        }
    }

    fn unary(&mut self, depth: u32) -> Result<Sk, ReparseError> {
        let mark = match &self.tok {
            Tok::Punct(p @ ("!" | "-" | "~" | "&" | "*")) => *p,
            Tok::Ident("out" | "ref") => "&",
            Tok::Ident("await") => "await",
            Tok::Punct("(") if self.lambda_ahead() => return self.lambda(depth),
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
        // `out T x` declares the variable it hands the address of.
        if mark == "&" && self.declaration_ahead_of(&[",", ")"]) {
            self.ty()?;
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
                e = match (field(name), received(e)) {
                    // The receiver is a reference: its fields are reached through it.
                    (Some(offset), Ok(this)) => Sk::Op("->", vec![this, Sk::Num(offset, Lit::Int)]),
                    (Some(offset), Err(e)) => Sk::Op(".f", vec![e, Sk::Num(offset, Lit::Int)]),
                    (None, Ok(e) | Err(e)) => member(e, name),
                };
            } else if self.eat("?.")? {
                // `x?.f_18`: the field where `x` refers to an object, else nothing.
                let name = self.ident()?;
                let base = received(e).unwrap_or_else(|e| e);
                e = match field(name) {
                    Some(offset) => Sk::Op("?.", vec![base, Sk::Num(offset, Lit::Int)]),
                    None => unread("a member asked of what may be null"),
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
                return Ok(received(e).unwrap_or_else(|e| e));
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
                    // `new T[] { a, b }`: the elements, each stored in its place.
                    if self.eat("]")? {
                        self.skip_braces()?;
                        return Ok(unread("an array written with its elements"));
                    }
                    let len = self.expr(depth)?;
                    self.expect("]")?;
                    Sk::Op("new[]", vec![Sk::Name(ty), len])
                } else {
                    let ctor = match self.is("{") {
                        true => None,
                        false => {
                            self.expect("(")?;
                            Some(self.list(")", depth)?)
                        }
                    };
                    // `new T(a) { f_8 = b }`: the fields stored after the constructor, which
                    // `new T { f_8 = b }` stores with no constructor run.
                    if self.is("{") {
                        let ctor = ctor.map_or_else(none, |args| Sk::Op(CTOR, args));
                        let mut items = vec![Sk::Name(ty), ctor];
                        items.extend(self.initialiser(depth)?);
                        return Ok(Sk::Op(INIT, items));
                    }
                    made(ty, ctor.unwrap_or_default())
                }
            }
            // `this.f_10` is a field of the object the receiver refers to, and `base.f_10` one a base type declares.
            Tok::Ident("this" | "base") if self.this.is_some() => {
                let this = self.this.clone().unwrap_or_default();
                Sk::Op("this", vec![Sk::Name(this)])
            }
            // ⚠ The type's name is not held: the printer spells a type's name by rules of its own.
            Tok::Ident("typeof") if self.is("(") => {
                self.bump()?;
                self.ty()?;
                self.expect(")")?;
                Sk::Op("typeof", Vec::new())
            }
            Tok::Ident(_) if self.is("=>") => self.lambda(depth)?,
            Tok::Ident(name) => Sk::Name(name.to_owned()),
            other => {
                let what = format!("an expression expected, not {}", describe(&other));
                return Err(ReparseError::Syntax { line, what });
            }
        })
    }
}

/// `e` without the 64-bit readings around it, which change no bit of a cell.
fn under_wide(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// `e` under the source readings around it.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// The objects an initialiser holds inside one another at most, as the printer writes them.
const NESTS: usize = 4;

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

/// The name a property's use is written under: the property an accessor's name names, else the callee's last name.
fn property_name(callee: &Callee) -> &str {
    let name = last_name(callee);
    super::passes::classes::accessor(name).map_or(name, |(_, property)| property)
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

/// The runtime's method that makes a `Type` of a type's token, by the path the printer knows it under.
const TYPE_OF: &str = "System.Type.GetTypeFromHandle";

/// Whether a call is the token of a type the program names.
fn type_token(callee: &Callee, args: &[Expr]) -> bool {
    matches!(
        (callee, args),
        (Callee::Intrinsic { name, ty: Some(_), .. }, []) if name == TYPE_TOKEN
    )
}

/// The C# keyword of a .NET primitive type, by the name a program states for it, as the source level writes it.
// A table of the language's own words; it must say what the printer's says.
fn keyword(name: &str) -> Option<&'static str> {
    Some(match name {
        "System.Boolean" => "bool",
        "System.Byte" => "byte",
        "System.SByte" => "sbyte",
        "System.Int16" => "short",
        "System.UInt16" => "ushort",
        "System.Int32" => "int",
        "System.UInt32" => "uint",
        "System.Int64" => "long",
        "System.UInt64" => "ulong",
        "System.Single" => "float",
        "System.Double" => "double",
        "System.Char" => "char",
        "System.String" => "string",
        "System.Object" => "object",
        _ => return None,
    })
}

/// Whether the statements of a case can run off their end: the last one neither leaves nor is an `if` both of whose arms leave.
// An explicit stack: an `if` in an arm nests as deep as the listing does.
fn falls(body: &[tree::Stmt]) -> bool {
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        match list.last() {
            Some(
                tree::Stmt::Return(_)
                | tree::Stmt::Break
                | tree::Stmt::Continue
                | tree::Stmt::Goto(_)
                | tree::Stmt::Unreachable,
            ) => {}
            Some(tree::Stmt::Sugar(sugar)) if matches!(**sugar, tree::Sugar::Throw(_)) => {}
            Some(tree::Stmt::If { then, els, .. }) if !then.is_empty() && !els.is_empty() => {
                open.push(then);
                open.push(els);
            }
            _ => return true,
        }
    }
    false
}

/// The bytes a value of `kind` takes, where the kind states them.
fn kind_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Bool | Kind::I8 | Kind::U8 => Some(1),
        Kind::I16 | Kind::U16 => Some(2),
        Kind::I32 | Kind::U32 | Kind::F32 | Kind::Hash => Some(4),
        Kind::I64
        | Kind::U64
        | Kind::F64
        | Kind::Pointer
        | Kind::Object(_)
        | Kind::String
        | Kind::Array(_) => Some(8),
        Kind::Unknown | Kind::Vector | Kind::Handle | Kind::Aggregate(_) | Kind::Value { .. } => {
            None
        }
    }
}

/// The bytes an element of an array of `kind` takes in the array, where the kind states them.
fn element_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Vector | Kind::Handle => Some(16),
        Kind::Aggregate(cells) | Kind::Value { cells, .. } => cells.checked_mul(8),
        scalar => kind_bytes(scalar),
    }
}

/// The operator a type declares under a method of this name, as true C# writes the call.
fn operator(name: &str) -> Option<BinOp> {
    Some(match name {
        "op_Equality" => BinOp::Eq,
        "op_Inequality" => BinOp::Ne,
        "op_Addition" => BinOp::Add,
        "op_Subtraction" => BinOp::Sub,
        "op_Multiply" => BinOp::Mul,
        "op_Division" => BinOp::Div,
        "op_Modulus" => BinOp::Rem,
        "op_LessThan" => BinOp::Lt,
        "op_LessThanOrEqual" => BinOp::Le,
        "op_GreaterThan" => BinOp::Gt,
        "op_GreaterThanOrEqual" => BinOp::Ge,
        _ => return None,
    })
}

/// Whether evaluating `e` calls or makes anything, which one statement would do once and two do twice.
fn calls(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(
            node,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    found
}

/// The name of the source form a group the printer writes as one statement of its own makes, where the reader does not hold that form yet.
fn group_form(kind: tree::Group) -> Option<&'static str> {
    Some(match kind {
        tree::Group::Collection => "an array written with its elements",
        tree::Group::Delegate => "a delegate made",
        tree::Group::Out => "an out argument",
        tree::Group::Deconstruct => "a deconstruction",
        tree::Group::Construct
        | tree::Group::Compound
        | tree::Group::Field { .. }
        | tree::Group::Concat => return None,
    })
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
    /// The passes on where the text is the source level's own: its constructs are held as it writes them; `None` for the readable form of old.
    sharp: Option<Options>,
    /// The parameter `this` stands for, where the text is a method's.
    this: Option<Var>,
}

impl Held<'_> {
    /// Whether the text is the source level's own and `pass` is on.
    fn on(&self, pass: Pass) -> bool {
        self.sharp.is_some_and(|options| options.on(pass))
    }

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

    /// Whether the text is the source level's own.
    fn sharp(&self) -> bool {
        self.sharp.is_some()
    }

    /// The cells a variable holds, as the printer counts them.
    fn cells(&self, var: Var) -> Option<u32> {
        cells_of(&Expr::Var(var), self.head)
    }

    /// Whether a variable is declared of a kind that refers to something: an object, a string, an array or a pointer.
    fn refers(&self, var: Var) -> bool {
        let kind = self.head.vars.get(&var).map(|info| &info.kind);
        matches!(
            kind,
            Some(Kind::Object(_) | Kind::String | Kind::Array(_) | Kind::Pointer)
        )
    }

    /// Whether evaluating `e` reads one cell of the frame and does nothing more: a variable of one cell, or one cell of a variable.
    fn one_cell(&self, e: &Expr) -> bool {
        match e {
            Expr::Var(var) => self.cells(*var) == Some(1),
            Expr::Cell { base, .. } => matches!(&**base, Expr::Var(_)),
            _ => false,
        }
    }

    /// A type's name as the source level writes it: a .NET primitive by its keyword.
    // ⚠ A generic type's instance the project names is written with its arguments, which no
    // function shows: such a name is held as it is listed, and the two then differ.
    fn type_name(&self, listed: &str) -> String {
        match self.sharp().then(|| keyword(listed)).flatten() {
            Some(keyword) => keyword.to_owned(),
            None => listed.to_owned(),
        }
    }

    /// The name a call is written under, in the readable form of old or at the source level.
    fn callee_name(&self, callee: &Callee) -> String {
        let listed = callee_name(callee, &self.names);
        if !self.sharp() {
            return listed;
        }
        match callee {
            // The type the slot is of, never the listing's wrapper class.
            Callee::Virtual { ty, method, .. } => format!("{}.{method}", self.type_name(ty)),
            Callee::Builtin { path, .. } => match path.rsplit_once('.') {
                Some((owner, member)) => format!("{}.{member}", self.type_name(owner)),
                None => listed,
            },
            Callee::Intrinsic { .. } => {
                let open = listed
                    .strip_suffix('>')
                    .and_then(|open| open.split_once('<'));
                let typed = open.map(|(head, ty)| format!("{head}<{}>", self.type_name(ty)));
                typed.unwrap_or(listed)
            }
            Callee::Local { .. } | Callee::Import { .. } | Callee::Native { .. } => listed,
        }
    }

    /// The form `extract_bytes(value, offset, bytes)` takes besides its name where the source level writes it as the field of a value it is: `None` where it writes it by name alone.
    // ⚠ Of a call the reader cannot tell whether it yields a reference, whose bytes the printer
    // writes by name: there the field form is held too.
    fn taken(&self, value: &Expr, offset: u32, bytes: u32) -> Option<Form> {
        if !self.sharp() || bytes == 0 {
            return None;
        }
        let (mut value, mut at, mut item) = (under_wide(value), offset, None);
        if let Expr::Cells(items) = value {
            match items.as_slice() {
                [one] => value = under_wide(one),
                _ => {
                    let cell = offset / 8;
                    let last = offset.checked_add(bytes - 1)? / 8;
                    let plain = items.iter().all(|item| self.one_cell(under_wide(item)));
                    if cell != last || !plain {
                        return None;
                    }
                    value = under_wide(items.get(usize::try_from(cell).ok()?)?);
                    item = usize::try_from(cell).ok();
                    at = offset % 8;
                }
            }
        }
        let whole = bytes == 8 && at == 0 && self.one_cell(value);
        // ⚠ A variable of one cell has no field the printer writes: its bytes are a shift.
        let own = match value {
            Expr::Var(var) => {
                self.this != Some(*var) && !self.refers(*var) && (whole || !self.single(*var))
            }
            Expr::Call { .. } => true,
            Expr::Sugar { kind, e } => {
                !matches!(kind, ExprSugar::Field { .. }) && matches!(&**e, Expr::Call { .. })
            }
            _ => false,
        };
        own.then_some(Form::Field {
            item,
            at: u64::from(at),
            whole,
        })
    }

    /// Whether a dot after `var` goes through what it holds, as the printer writes it with `fields` on: a variable of one cell.
    fn single(&self, var: Var) -> bool {
        self.sharp() && self.on(Pass::Fields) && reads_through(self.head, var)
    }

    /// `extract_bytes(value, offset, bytes)` of a variable of one cell as the printer writes it, the variable shifted down to the bytes, or by name past the bytes its kind holds; `None` where the value is no such variable.
    fn shifted(&self, value: &Expr, offset: u32, bytes: u32) -> Option<Sk> {
        if !self.sharp() || bytes == 0 {
            return None;
        }
        let (mut value, mut at) = (under_wide(value), offset);
        if let Expr::Cells(items) = value {
            match items.as_slice() {
                [one] => value = under_wide(one),
                _ => {
                    let cell = offset / 8;
                    let last = offset.checked_add(bytes - 1)? / 8;
                    let plain = items.iter().all(|item| self.one_cell(under_wide(item)));
                    if cell != last || !plain {
                        return None;
                    }
                    value = under_wide(items.get(usize::try_from(cell).ok()?)?);
                    at = offset % 8;
                }
            }
        }
        let Expr::Var(var) = value else {
            return None;
        };
        if self.this == Some(*var) || self.refers(*var) || !self.single(*var) {
            return None;
        }
        // The whole of a cell is the cell, as the printer writes it before any shift.
        if bytes == 8 && at == 0 && self.one_cell(value) {
            return None;
        }
        let kind = self
            .head
            .vars
            .get(var)
            .map_or(Kind::Unknown, |info| info.kind.clone());
        let held: u64 = match kind {
            Kind::I8 | Kind::U8 => 1,
            Kind::I16 | Kind::U16 => 2,
            Kind::I32 | Kind::U32 => 4,
            Kind::I64 | Kind::U64 | Kind::Unknown => 8,
            _ => return None,
        };
        let name = Sk::Name(self.var(*var));
        Some(match (u64::from(at) + u64::from(bytes) > held, at) {
            (true, _) => return None,
            (false, 0) => name,
            (false, at) => Sk::Op(">>", vec![name, num(at * 8)]),
        })
    }

    /// The two variables `bytes_equal_N(&a, &b)` compares whole, which the source level writes `a == b`.
    fn whole_equality(&self, name: &str, args: &[Expr]) -> Option<(Var, Var)> {
        let bytes: u32 = name.strip_prefix("bytes_equal_")?.parse().ok()?;
        let [Expr::AddrOf { var: l, cell: 0 }, Expr::AddrOf { var: r, cell: 0 }] = args else {
            return None;
        };
        // Three floats fill twelve bytes of their two cells.
        let whole = |var: &Var| {
            let vector = (self.head.vars.get(var)).is_some_and(|info| info.kind == Kind::Vector);
            let cells = self.cells(*var).filter(|cells| *cells >= 2);
            let held = cells.map(|cells| if vector { 12 } else { cells.saturating_mul(8) });
            held == Some(bytes)
        };
        (self.on(Pass::Operators) && whole(l) && whole(r)).then_some((*l, *r))
    }

    /// The variable a clear of `bytes` bytes at `addr` empties whole, which the source level writes `x = default`.
    fn whole_clear(&self, addr: &Expr, bytes: u32) -> Option<Var> {
        let Expr::AddrOf { var, cell: 0 } = addr else {
            return None;
        };
        let cells = self.cells(*var)?;
        let whole = u64::from(bytes) == u64::from(cells) * 8;
        (self.sharp() && whole).then_some(*var)
    }

    /// `x = new T(a) { f_8 = b }`: the statement a construct group reads as at the source level, and how many of its parts it holds; `None` where it is written as its parts.
    fn construction(&self, parts: &[Stmt], d: u32) -> Result<Option<(Sk, usize)>, ReparseError> {
        let Some(Stmt::Assign { target, .. }) = parts.first() else {
            return Ok(None);
        };
        let Some((value, held)) = self.filled_object(parts, 0, d)? else {
            return Ok(None);
        };
        Ok(Some((assign(Sk::Name(self.var(*target)), value), held)))
    }

    /// `new T(a) { F = b, G = new U { H = c } }` of the parts that make an object and fill it, with how many parts it holds.
    fn filled_object(
        &self,
        parts: &[Stmt],
        depth: usize,
        d: u32,
    ) -> Result<Option<(Sk, usize)>, ReparseError> {
        let Some((Stmt::Assign { target, value }, rest)) = parts.split_first() else {
            return Ok(None);
        };
        let Expr::NewObject(ty) = bare(value) else {
            return Ok(None);
        };
        // The constructor is the call that stands second, with the object first among its arguments.
        let passed = match rest.first() {
            Some(Stmt::Eval(call)) => {
                let Expr::Call { args, .. } = bare(call) else {
                    return Ok(None);
                };
                match args.first() {
                    Some(object) if *bare(under_wide(object)) == Expr::Var(*target) => Some(args),
                    _ => return Ok(None),
                }
            }
            _ => None,
        };
        if passed.is_some() && !self.on(Pass::Constructors) {
            return Ok(None);
        }
        let stores = rest
            .get(usize::from(passed.is_some())..)
            .unwrap_or_default();
        let mut fields = Vec::new();
        let mut taken = 0;
        if self.on(Pass::Initialisers) {
            while taken < stores.len() {
                let held = match self.nested(stores, taken, *target, depth, d)? {
                    Some(found) => Some(found),
                    None => (self.initialised(&stores[taken], *target, d)?).map(|field| (field, 1)),
                };
                match held {
                    Some((field, held)) => {
                        fields.push(field);
                        taken += held;
                    }
                    // One store that is no plain field of the object leaves them all statements.
                    None => {
                        fields.clear();
                        taken = 0;
                        break;
                    }
                }
            }
        }
        let ty = self.type_name(&ty.name);
        let value = match (passed, fields.is_empty()) {
            (None, true) => return Ok(None),
            (Some(args), true) => made(ty, self.list(args.get(1..).unwrap_or_default(), d)?),
            (passed, false) => {
                let ctor = match passed {
                    Some(args) => Sk::Op(CTOR, self.list(args.get(1..).unwrap_or_default(), d)?),
                    None => none(),
                };
                let mut items = vec![Sk::Name(ty), ctor];
                items.extend(fields);
                Sk::Op(INIT, items)
            }
        };
        Ok(Some((value, 1 + usize::from(passed.is_some()) + taken)))
    }

    /// `F = new U { .. }` for an object made at `stores[at]`, filled by the parts after it and stored whole into a field of the object `object` holds; with how many parts that is.
    // ⚠ An array made so is a form not read back: the parts it holds are then not counted.
    fn nested(
        &self,
        stores: &[Stmt],
        at: usize,
        object: Var,
        depth: usize,
        d: u32,
    ) -> Result<Option<(Sk, usize)>, ReparseError> {
        if depth >= NESTS {
            return Ok(None);
        }
        let Some(Stmt::Assign {
            target: inner,
            value,
        }) = stores.get(at)
        else {
            return Ok(None);
        };
        if !matches!(bare(value), Expr::NewObject(_) | Expr::NewArray { .. }) {
            return Ok(None);
        }
        // The store of the inner object into a field of the outer one closes its parts.
        let closes = |stmt: &Stmt| match stmt {
            Stmt::Store { addr, value, .. } => {
                *bare(under_wide(addr)) == Expr::Var(object)
                    && *bare(under_wide(value)) == Expr::Var(*inner)
            }
            _ => false,
        };
        let Some(end) = stores
            .get(at..)
            .and_then(|rest| rest.iter().position(closes))
        else {
            return Ok(None);
        };
        let end = at + end;
        let Some(Stmt::Store { offset, .. }) = stores.get(end) else {
            return Ok(None);
        };
        if *offset == 0 {
            return Ok(None);
        }
        let parts = &stores[at..end];
        let value = match bare(value) {
            Expr::NewObject(_) if parts.len() == 1 => self.sk(value, d)?,
            Expr::NewObject(_) => match self.filled_object(parts, depth + 1, d)? {
                Some((value, held)) if held == parts.len() => value,
                _ => return Ok(None),
            },
            _ => unread("an array written with its elements"),
        };
        Ok(Some((assign(num(*offset), value), end + 1 - at)))
    }

    /// `F = value` for a store to a field of the object `object` holds, as an initialiser writes it; `None` for one the printer writes no field of.
    fn initialised(&self, store: &Stmt, object: Var, d: u32) -> Result<Option<Sk>, ReparseError> {
        let (addr, offset) = match store {
            Stmt::Store { addr, offset, .. } => (addr, offset),
            // Bytes of fewer cells than the value stored are written as a call, no field.
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } if bytes.is_multiple_of(8) && cells_of(value, self.head) == Some(bytes / 8) => {
                (addr, offset)
            }
            _ => return Ok(None),
        };
        if *bare(under_wide(addr)) != Expr::Var(object) || *offset == 0 {
            return Ok(None);
        }
        // What follows the place is what the field takes.
        let value = match self.simple(store, d)? {
            Sk::Op("=", mut pair) if pair.len() == 2 => pair.remove(1),
            _ => return Ok(None),
        };
        Ok(Some(assign(num(*offset), value)))
    }

    /// Whether the source level writes nothing for `arm`: it is empty, or holds only what the compiler added.
    fn unwritten(&self, arm: &[tree::Stmt]) -> bool {
        let source = self
            .sharp
            .is_some_and(|options| options.level == Level::Source);
        let hidden = |stmt: &tree::Stmt| match stmt {
            tree::Stmt::Sugar(sugar) => source && matches!(**sugar, tree::Sugar::Hidden(_)),
            _ => false,
        };
        self.sharp() && arm.iter().all(hidden)
    }

    /// The statement the source level ends `case` with where it would run off its end, which C# refuses: `break` from the last case, `goto case` the next one's first label from any other that holds a statement.
    fn closer(&self, case: &tree::Case, next: Option<&tree::Case>) -> Option<Node> {
        if !self.sharp() || !falls(&case.body) {
            return None;
        }
        let Some(next) = next else {
            return Some(Node::held("break", Vec::new(), Vec::new()));
        };
        if case.body.is_empty() {
            return None;
        }
        let label = match next.labels.first()? {
            Some(value) => Sk::Num(*value as u64, Lit::Int),
            None => Sk::Name("default".to_owned()),
        };
        Some(Node::held("goto case", vec![label], Vec::new()))
    }

    /// Whether the source level may write this call as `l == r` of two boxed values or as a cast of one, which the project decides by the type boxed.
    fn boxed_call(&self, callee: &Callee, args: &[Expr]) -> bool {
        let boxed = |e: &Expr| {
            matches!(
                under_wide(e),
                Expr::Call { callee: Callee::Intrinsic { name, .. }, args, .. }
                    if args.len() == 1 && name.starts_with("box<") && name.ends_with('>')
            )
        };
        let Callee::Builtin { path, .. } = callee else {
            return false;
        };
        let read = match args {
            [l, r] => path == "System.Object.Equals" && boxed(l) && boxed(r),
            [one] => path == "System.Convert.ToInt32" && boxed(one),
            _ => false,
        };
        read && self.on(Pass::Boxing)
    }

    /// Whether `e` is `l > 0` over unsigned cells, which the source level writes `l != null` where `l` refers to something.
    fn unsigned_above_zero(&self, e: &Expr) -> bool {
        matches!(
            e,
            Expr::Binary { op: BinOp::Gt, at: Kind::U64, r, .. } if self.sharp() && is_zero(r)
        )
    }

    /// `l > 0` over unsigned cells as the source level writes it: as the truth of `l` where `l` refers to something, as the comparison where it cannot, and as either where the reader cannot tell.
    fn above_zero(&self, l: &Expr, d: u32) -> Result<Sk, ReparseError> {
        let value = self.sk(l, d)?;
        let mut inner = under_wide(l);
        while let Expr::Sugar { e, .. } = inner {
            inner = &**e;
        }
        let cast = matches!(
            inner,
            Expr::Call { callee: Callee::Intrinsic { name, .. }, .. }
                if name.starts_with("try_cast<") || name.starts_with("cast<")
        );
        let refers = match under_wide(l) {
            // What a global holds is the project's to say.
            Expr::Var(Var::Global { .. }) => None,
            Expr::Var(var) => Some(self.refers(*var)),
            Expr::Binary { at, .. } => Some(matches!(
                at,
                Kind::Object(_) | Kind::String | Kind::Array(_) | Kind::Pointer
            )),
            Expr::Load { .. }
            | Expr::LoadBytes { .. }
            | Expr::ExtractBytes { .. }
            | Expr::Unary { .. }
            | Expr::Truth(_)
            | Expr::Cast { .. } => Some(false),
            _ => None,
        };
        Ok(match (cast, refers) {
            (true, _) | (false, Some(true)) => truth(value),
            (false, Some(false)) => binary(">", value, num(0)),
            (false, None) => Sk::Op(ABOVE_ZERO, vec![value]),
        })
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
        // The top byte of a reference's offset holds its flags, which the printer leaves unsaid.
        let at = at & 0x00FF_FFFF;
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
            // True C# writes one cell gathered alone as that cell.
            Expr::Cells(items) if self.sharp() && items.len() == 1 => self.sk(&items[0], d)?,
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
            Expr::Binary { l, .. } if self.unsigned_above_zero(e) => self.above_zero(l, d)?,
            Expr::Binary { .. } => {
                // A long sum is a chain down its left side: walked, not recursed into.
                let mut spine = Vec::new();
                let mut left = e;
                while let Expr::Binary { op, l, r, .. } = left {
                    let logical = matches!(op, BinOp::LogicalAnd | BinOp::LogicalOr);
                    // The first is none: the arm before takes it.
                    if logical || self.unsigned_above_zero(left) {
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
            Expr::Load { ty, addr, offset } => {
                self.place_of(addr, *offset, false, Some(ty.bytes()), d)?
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if bytes.is_multiple_of(8) => self.place_of(addr, *offset, false, Some(*bytes), d)?,
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
            } => {
                if let Some(shifted) = self.shifted(value, *offset, *bytes) {
                    return Ok(shifted);
                }
                let args = vec![self.sk(value, d)?, num(*offset), num(*bytes)];
                match self.taken(value, *offset, *bytes) {
                    Some(form) => Sk::Call {
                        name: "extract_bytes".to_owned(),
                        form,
                        args,
                    },
                    None => call("extract_bytes", args),
                }
            }
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
            Expr::NewObject(ty) => made(self.type_name(&ty.name), Vec::new()),
            Expr::NewArray { ty, len, .. } => Sk::Op(
                "new[]",
                vec![Sk::Name(self.type_name(&ty.name)), self.sk(len, d)?],
            ),
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
        if self.sharp.is_some() {
            return self.sharp_sugared(kind, e, d);
        }
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

    /// A field read through `addr` as the source level writes it: a variable's own bytes as the place they are, `(&x)->f_N` where `x` holds one cell and `x.f_N` where it holds several.
    fn field_sk(&self, addr: &Expr, offset: u32, d: u32) -> Result<Sk, ReparseError> {
        Ok(match addr {
            Expr::AddrOf { var, cell: 0 } => {
                let name = Sk::Name(self.var(*var));
                match self.single(*var) {
                    true => Sk::Op("->", vec![Sk::Op("&", vec![name]), num(offset)]),
                    false => Sk::Op(".f", vec![name, num(offset)]),
                }
            }
            _ => Sk::Op(".f", vec![self.sk(addr, d)?, num(offset)]),
        })
    }

    /// `e` as the source level reads it as the construct `kind`: as the readable form of old does where the two write alike, and as what it holds where the construct's pass is off.
    fn sharp_sugared(&self, kind: ExprSugar, e: &Expr, d: u32) -> Result<Sk, ReparseError> {
        let pass = match kind {
            ExprSugar::Method => Some(Pass::Receivers),
            ExprSugar::Field { .. } => Some(Pass::Fields),
            ExprSugar::Property => Some(Pass::Properties),
            ExprSugar::Operator => Some(Pass::Operators),
            ExprSugar::Lambda => Some(Pass::Lambdas),
            ExprSugar::Coalesce | ExprSugar::NullConditional => Some(Pass::NullOps),
            ExprSugar::Concat => Some(Pass::Concat),
            ExprSugar::Out | ExprSugar::Ref => Some(Pass::RefOut),
            ExprSugar::Tuple => Some(Pass::Tuples),
            ExprSugar::Convert | ExprSugar::TypeOf => None,
        };
        if pass.is_some_and(|pass| !self.on(pass)) {
            return self.sk(e, d);
        }
        Ok(match (kind, e) {
            (ExprSugar::Field { .. }, Expr::LoadBytes { addr, offset, .. }) => {
                self.field_sk(addr, *offset, d)?
            }
            (
                ExprSugar::Operator,
                Expr::Call {
                    callee: Callee::Builtin { path, .. },
                    args,
                    ..
                },
            ) => {
                let op = path.rsplit('.').next().and_then(operator);
                match (op, args.as_slice()) {
                    (Some(op), [l, r]) => binary(op.symbol(), self.sk(l, d)?, self.sk(r, d)?),
                    _ => self.sk(e, d)?,
                }
            }
            // `x?.f_18`: a whole cell of a field asked of what may refer to nothing.
            (ExprSugar::NullConditional, Expr::Select { then, .. }) => {
                let mut field = under_wide(then);
                if let Expr::Sugar {
                    kind: ExprSugar::Field { .. },
                    e: inner,
                } = field
                {
                    field = &**inner;
                }
                match field {
                    Expr::Load {
                        ty: Scalar::U64,
                        addr,
                        offset,
                    } => Sk::Op("?.", vec![self.sk(under_wide(addr), d)?, num(*offset)]),
                    _ => self.sk(e, d)?,
                }
            }
            // `(T)x`: a conversion to the type the intrinsic names, which states nothing of the bits.
            (
                ExprSugar::Convert,
                Expr::Call {
                    callee: Callee::Intrinsic { name, ty, .. },
                    args,
                    ..
                },
            ) => {
                let to = ty.is_some()
                    || name
                        .split_once('<')
                        .is_some_and(|(_, rest)| rest.ends_with('>'));
                match (to, args.as_slice()) {
                    (true, [value]) => self.sk(value, d)?,
                    _ => self.sk(e, d)?,
                }
            }
            // The rest the source level writes as the form of old does.
            (ExprSugar::Method, Expr::Call { callee, args, .. }) if !args.is_empty() => called(
                member(self.sk(&args[0], d)?, last_name(callee)),
                self.list(&args[1..], d)?,
            ),
            // A static property of the runtime's own types, by the type that declares it.
            (
                ExprSugar::Property,
                Expr::Call {
                    callee: callee @ Callee::Builtin { path, .. },
                    args,
                    ..
                },
            ) if args.is_empty() => match path.rsplit_once('.') {
                Some((owner, _)) => Sk::Name(format!(
                    "{}.{}",
                    self.type_name(owner),
                    property_name(callee)
                )),
                None => self.sk(e, d)?,
            },
            // The source level names a property by the accessor rule its declaration takes.
            (ExprSugar::Property, Expr::Call { callee, args, .. }) if args.len() == 1 => {
                member(self.sk(&args[0], d)?, property_name(callee))
            }
            (ExprSugar::Property, Expr::Call { callee, args, .. }) if args.len() == 2 => assign(
                member(self.sk(&args[0], d)?, property_name(callee)),
                self.sk(&args[1], d)?,
            ),
            (ExprSugar::Field { .. }, Expr::Load { addr, offset, .. }) => {
                self.field_sk(addr, *offset, d)?
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
        // The token of a type is `typeof` of it; true C# with operators on says the token alone by its handle.
        if type_token(callee, args) {
            let token = Sk::Op("typeof", Vec::new());
            return Ok(match self.on(Pass::Operators) {
                true => member(token, "TypeHandle"),
                false => token,
            });
        }
        // There the runtime's method that makes a `Type` of a token is `typeof` itself.
        if let (true, Callee::Builtin { path, .. }, [token]) =
            (self.on(Pass::Operators), callee, args)
        {
            let mut token = token;
            while let Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } = token
            {
                token = &**inner;
            }
            if let (true, Expr::Call { callee, args, .. }) = (path == TYPE_OF, token) {
                if type_token(callee, args) {
                    return Ok(Sk::Op("typeof", Vec::new()));
                }
            }
        }
        if let Callee::Intrinsic { name, .. } = callee {
            if let Some(spelled) = self.intrinsic(name, args, d)? {
                return Ok(spelled);
            }
        }
        if self.boxed_call(callee, args) {
            return Ok(unread("a boxed value compared or converted"));
        }
        let name = unquoted(self.callee_name(callee));
        let passed = self.list(args, d)?;
        let form = match (callee, args) {
            // ⚠ The project says which function is a type's constructor, which no function shows:
            // a script function run on an object made in the call reads either way.
            (Callee::Local { .. } | Callee::Import { .. }, [Expr::NewObject(ty), ..])
                if self.on(Pass::Constructors) =>
            {
                Some(Form::Made(self.type_name(&ty.name)))
            }
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
        if let Some((l, r)) = self.whole_equality(name, args) {
            let (l, r) = (Sk::Name(self.var(l)), Sk::Name(self.var(r)));
            return Ok(Some(binary("==", l, r)));
        }
        Ok(Some(match args {
            [target, passed @ ..] if name == "invoke_delegate" => {
                called(self.sk(target, d)?, self.list(passed, d)?)
            }
            [array] if name == "array_length" => member(self.sk(array, d)?, "Length"),
            // A cast that must hold, `(T)value`: what it states of the value is no operand.
            [value] if name.starts_with("cast<") && name.ends_with('>') => self.sk(value, d)?,
            [value] => match name
                .strip_prefix("try_cast<")
                .and_then(|t| t.strip_suffix('>'))
            {
                Some(ty) => Sk::Op("as", vec![self.sk(value, d)?, Sk::Name(self.type_name(ty))]),
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
        self.place_of(addr, offset, partial, None, d)
    }

    /// [`Held::place`] for an access of `bytes` bytes, where the access states them.
    // ⚠ At the source level fewer bytes than a variable of several cells holds are a part of it,
    // `x.f_0`, as a narrow store is of a cell; and what stands at an element's address is the
    // element where the access is as wide as one.
    fn place_of(
        &self,
        addr: &Expr,
        offset: u32,
        partial: bool,
        bytes: Option<u32>,
        d: u32,
    ) -> Result<Sk, ReparseError> {
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
        // ⚠ With `fields` on, a dot after a variable of one cell goes through what it holds, as
        // the printer writes it, and the bytes of the variable itself are `(&x)->f_N`.
        let held = matches!(under_wide(base), Expr::Var(var) if self.this != Some(*var) && self.single(*var));
        let mut boxed = matches!(base, Expr::AddrOf { var, cell: 0 } if self.single(*var));
        let (mut place, variable) = match base {
            Expr::AddrOf { var, cell: 0 } => (Sk::Name(self.var(*var)), true),
            Expr::AddrOf { var, cell } => (index_of(Sk::Name(self.var(*var)), num(*cell)), true),
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } if self.on(Pass::Fields)
                && !embedded
                && offset == 0
                && name == "array_element_address"
                && args.len() == 2 =>
            {
                return self.at_element(&args[0], &args[1], bytes, d);
            }
            pointer if !embedded && offset == 0 => {
                return Ok(Sk::Op("*", vec![self.sk(pointer, d)?]));
            }
            pointer => (self.sk(pointer, d)?, false),
        };
        let part = self.sharp()
            && match (base, bytes) {
                (Expr::AddrOf { var, cell: 0 }, Some(bytes)) => match self.cells(*var) {
                    Some(cells) => cells >= 2 && u64::from(bytes) < u64::from(cells) * 8,
                    None => true,
                },
                _ => false,
            };
        // `->` goes through a pointer and `.` stays in place.
        let mut through = !variable;
        for (n, field) in fields.iter().enumerate().rev() {
            let unsaid = *field == 0 && n == 0 && (embedded || (variable && !partial && !part));
            if !unsaid {
                if std::mem::take(&mut boxed) {
                    place = Sk::Op("&", vec![place]);
                    through = true;
                }
                let mark = if std::mem::take(&mut through) && !held {
                    "->"
                } else {
                    ".f"
                };
                place = Sk::Op(mark, vec![place, Sk::Num(*field, Lit::Int)]);
            }
        }
        Ok(place)
    }

    /// What stands at the address of element `index` of `array`, read `bytes` wide, as the source level writes it: the element where the access is as wide as one, else the bytes at its address.
    fn at_element(
        &self,
        array: &Expr,
        index: &Expr,
        bytes: Option<u32>,
        d: u32,
    ) -> Result<Sk, ReparseError> {
        let pair = vec![self.sk(array, d)?, self.sk(index, d)?];
        let wide = match array {
            Expr::Var(Var::Global { .. }) => None,
            Expr::Var(var) => match self.head.vars.get(var).map(|info| &info.kind) {
                Some(Kind::Array(of)) => Some(element_bytes(of)),
                _ => Some(None),
            },
            _ => None,
        };
        let element = |pair: Vec<Sk>| Sk::Op("[]", pair);
        let at = |pair: Vec<Sk>| {
            let address = Sk::Op("&", vec![element(pair)]);
            Sk::Op("->", vec![address, num(0)])
        };
        Ok(match (bytes, wide) {
            (None, _) => at(pair),
            (Some(bytes), Some(wide)) if wide == Some(bytes) => element(pair),
            (Some(_), Some(_)) => at(pair),
            (Some(_), None) => Sk::Op(AT_ELEMENT, pair),
        })
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
                let wide = Some(kind.bytes());
                let place = self.place_of(addr, *offset, narrow.is_some(), wide, d)?;
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
                let place = self.place_of(addr, *offset, true, Some(*bytes), d)?;
                assign(place, self.sk(value, d)?)
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
            // True C# writes a clear of every byte of a variable as the variable made its default.
            Stmt::ZeroBytes { addr, bytes } => match self.whole_clear(addr, *bytes) {
                Some(var) => assign(Sk::Name(self.var(var)), Sk::Op("default", Vec::new())),
                None => call("zero_bytes", vec![self.sk(addr, d)?, num(*bytes)]),
            },
            Stmt::StepI32 { addr, offset, down } => {
                let mark = if *down { "--" } else { "++" };
                Sk::Op(mark, vec![self.place_of(addr, *offset, true, Some(4), d)?])
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
            // Only the form of old pairs the two itself: the source level writes what a pass made one.
            let pair = match self.sharp() {
                true => None,
                false => self.constructed(stmt, stmts.get(n), d)?,
            };
            if let Some(pair) = pair {
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
                // An `else` that is one `if` is written `else if`, however long the chain.
                let (mut word, mut cond, mut then, mut els) = ("if", cond, then, els);
                loop {
                    let test = truth(self.sk(cond, d)?);
                    // At the source level an arm nothing is written for is no `else`, and where
                    // it is the first the other stands under the test turned round.
                    match (self.unwritten(then), self.unwritten(els)) {
                        (true, false) => {
                            out.push(Node::held(word, vec![not(test)], vec![self.nodes(els, d)?]));
                            return Ok(());
                        }
                        (_, true) => {
                            out.push(Node::held(word, vec![test], vec![self.nodes(then, d)?]));
                            return Ok(());
                        }
                        (false, false) => {
                            out.push(Node::held(word, vec![test], vec![self.nodes(then, d)?]));
                        }
                    }
                    match els.as_slice() {
                        [] => return Ok(()),
                        [tree::Stmt::If {
                            cond: c,
                            then: t,
                            els: e,
                        }] => (word, cond, then, els) = ("else if", c, t, e),
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
                for (n, case) in cases.iter().enumerate() {
                    let labels = case.labels.iter().map(|label| match label {
                        Some(value) => Sk::Num(*value as u64, Lit::Int),
                        None => Sk::Name("default".to_owned()),
                    });
                    let labels: Vec<Sk> = labels.collect();
                    let mut body = self.nodes(&case.body, d)?;
                    body.extend(self.closer(case, cases.get(n + 1)));
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

    /// A source construct as the text writes it: by the form of old, or as the source level writes it where the text is its own.
    fn sugar(&self, sugar: &tree::Sugar, out: &mut Vec<Node>, d: u32) -> Result<(), ReparseError> {
        match self.sharp {
            Some(options) => self.sharp_sugar(sugar, options, out, d),
            None => self.old_sugar(sugar, out, d),
        }
    }

    /// A source construct as the source level writes it: by its keyword where its pass is on and the reader holds the form, by the form of old where the two write alike, else as the statements it stands for.
    // ⚠ A form the reader does not hold yet is one node that says so: the comparison stops there.
    fn sharp_sugar(
        &self,
        sugar: &tree::Sugar,
        options: Options,
        out: &mut Vec<Node>,
        d: u32,
    ) -> Result<(), ReparseError> {
        let on = |pass: Pass| options.on(pass);
        match sugar {
            // What the compiler added and no source wrote shows only below the source level.
            tree::Sugar::Hidden(_) if options.level == Level::Source => {}
            tree::Sugar::Hidden(stmts) => out.extend(self.nodes(stmts, d)?),
            tree::Sugar::Group { kind, parts } => self.group(*kind, parts, out, d)?,
            tree::Sugar::Await { after, .. } if on(Pass::Await) => {
                let Some(read) = async_await::reading(sugar) else {
                    out.push(unread_node("await"));
                    return Ok(());
                };
                self.simples(read.shown, out, d)?;
                let awaited = self.sk(read.awaited, d)?;
                match read.taken {
                    // The one statement that asks the awaiter for its result takes what the `await` yields.
                    async_await::Taken::Assigned(target) => {
                        let waited = Sk::Op("await", vec![awaited]);
                        let stated = assign(Sk::Name(self.var(target)), waited);
                        out.push(Node::held("", vec![stated], Vec::new()));
                    }
                    async_await::Taken::Dropped => {
                        out.push(Node::held("await", vec![awaited], Vec::new()));
                    }
                    async_await::Taken::Stored(_) | async_await::Taken::Nothing => {
                        out.push(Node::held("await", vec![awaited], Vec::new()));
                        self.simples(after, out, d)?;
                    }
                }
            }
            tree::Sugar::Yield { value, .. } if on(Pass::Iterators) => match value.as_slice() {
                // The one statement that keeps what is yielded says what that is.
                [Stmt::Assign { value: kept, .. } | Stmt::Store { value: kept, .. }] => {
                    let yielded = vec![self.sk(kept, d)?];
                    out.push(Node::held("yield", yielded, Vec::new()));
                }
                _ => {
                    self.simples(value, out, d)?;
                    out.push(Node::held("yield", Vec::new(), Vec::new()));
                }
            },
            // Whether its item is declared by it is where the printer places declarations.
            tree::Sugar::Foreach { .. } if on(Pass::Foreach) => out.push(unread_node("foreach")),
            tree::Sugar::Guarded {
                kind: tree::Guard::Using,
                ..
            } if on(Pass::Using) => self.old_sugar(sugar, out, d)?,
            tree::Sugar::Guarded {
                kind: tree::Guard::Lock,
                ..
            } if on(Pass::Lock) => self.old_sugar(sugar, out, d)?,
            tree::Sugar::Try { .. } | tree::Sugar::Throw(_) if on(Pass::Try) => {
                self.old_sugar(sugar, out, d)?;
            }
            tree::Sugar::SwitchOn { .. } if on(Pass::SwitchOn) => {
                out.push(unread_node("a switch on a chain of tests"));
            }
            tree::Sugar::Event { .. } if on(Pass::Events) => out.push(unread_node("an event")),
            tree::Sugar::Cached { .. } if on(Pass::Lambdas) => out.push(unread_node("??=")),
            _ => self.lowered(sugar, out, d)?,
        }
        Ok(())
    }

    /// A group as the source level writes it: the one statement it reads as, where the reader holds that form.
    fn group(
        &self,
        kind: tree::Group,
        parts: &[Stmt],
        out: &mut Vec<Node>,
        d: u32,
    ) -> Result<(), ReparseError> {
        if let tree::Group::Construct = kind {
            match self.construction(parts, d)? {
                Some((stated, held)) => {
                    out.push(Node::held("", vec![stated], Vec::new()));
                    self.simples(parts.get(held..).unwrap_or_default(), out, d)?;
                }
                None => self.simples(parts, out, d)?,
            }
            return Ok(());
        }
        if let Some(form) = group_form(kind) {
            out.push(unread_node(form));
            return Ok(());
        }
        let read = match kind {
            tree::Group::Compound if self.on(Pass::Compound) => self.compound(parts, d)?,
            tree::Group::Field { .. } if self.on(Pass::Fields) => self.field_store(parts, d)?,
            _ => None,
        };
        match read {
            Some(node) => out.push(node),
            None => self.simples(parts, out, d)?,
        }
        Ok(())
    }

    /// `place op= value`, `place++`: the one statement a compound group is, where it has the shape the printer writes so; `None` where it is written as its part.
    fn compound(&self, parts: &[Stmt], d: u32) -> Result<Option<Node>, ReparseError> {
        let [part] = parts else {
            return Ok(None);
        };
        let (Stmt::Assign { value, .. } | Stmt::Store { value, .. }) = part else {
            return Ok(None);
        };
        let Expr::Binary { op, l, r, .. } = value else {
            return Ok(None);
        };
        if op.is_logical() {
            return Ok(None);
        }
        // What the operator reads on its left, under its readings, and whether a field it is read as.
        let (mut read, mut owned) = (&**l, false);
        loop {
            match read {
                Expr::Cast { e, .. } => read = &**e,
                Expr::Sugar { kind, e } => {
                    owned |= matches!(kind, ExprSugar::Field { .. });
                    read = &**e;
                }
                _ => break,
            }
        }
        let mut typed = true;
        let place = match (part, read) {
            (Stmt::Assign { target, .. }, Expr::Var(var)) if var == target => {
                Sk::Name(self.var(*target))
            }
            (
                Stmt::Store {
                    kind, addr, offset, ..
                },
                Expr::Load {
                    addr: from,
                    offset: read_at,
                    ..
                },
            ) if read_at == offset && **from == *addr && !calls(addr) => {
                match (owned, kind.bytes()) {
                    (true, _) => Sk::Op(".f", vec![self.sk(addr, d)?, num(*offset)]),
                    // A place no type declares says no width: only a whole cell reads as one.
                    (false, 8) => {
                        typed = false;
                        self.place_of(addr, *offset, false, Some(8), d)?
                    }
                    (false, _) => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        let one = matches!(&**r, Expr::Const(c) if c.bits == 1 && !c.kind.is_float());
        let stated = match (op, one && typed) {
            (BinOp::Add, true) => Sk::Op("++", vec![place]),
            (BinOp::Sub, true) => Sk::Op("--", vec![place]),
            _ => assign(place.clone(), binary(op.symbol(), place, self.sk(r, d)?)),
        };
        Ok(Some(Node::held("", vec![stated], Vec::new())))
    }

    /// `base.f_10 = value`: the one store a field group is, through a variable; a store of whole cells, or through a field, is a form not held yet.
    fn field_store(&self, parts: &[Stmt], d: u32) -> Result<Option<Node>, ReparseError> {
        let named = || Some(unread_node("a field stored by its name"));
        let [store] = parts else {
            return Ok(None);
        };
        let (kind, addr, offset, value) = match store {
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => (kind, addr, offset, value),
            Stmt::StoreBytes { offset, .. } if *offset != 0 => return Ok(named()),
            _ => return Ok(None),
        };
        if *offset == 0 {
            return Ok(None);
        }
        let base = match addr {
            Expr::Var(base) => *base,
            Expr::Sugar {
                kind: ExprSugar::Field { .. },
                ..
            } => return Ok(named()),
            _ => return Ok(None),
        };
        let value = match kind {
            StoreKind::U8 => self.stored(Scalar::U8, value, d)?,
            StoreKind::U16 => self.stored(Scalar::U16, value, d)?,
            StoreKind::I32 => self.stored(Scalar::I32, value, d)?,
            StoreKind::U64 | StoreKind::Ref => self.sk(value, d)?,
        };
        let place = Sk::Op(".f", vec![Sk::Name(self.var(base)), num(*offset)]);
        Ok(Some(Node::held("", vec![assign(place, value)], Vec::new())))
    }

    /// A source construct: its keyword and its blocks where its parts say what to write.
    fn old_sugar(
        &self,
        sugar: &tree::Sugar,
        out: &mut Vec<Node>,
        d: u32,
    ) -> Result<(), ReparseError> {
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
