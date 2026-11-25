//! Reading a lifted listing: the `.ysc.cs` text a decompiler of GTA VI's scripts writes, one C#
//! method per script function. Nothing else in the decompiler knows this syntax.
//!
//! ⚠ **The declarations at the end are read first.** A body calls `Native.NS.NAME(..)` or
//! `Operations.name(..)`, and only the declaration classes after the last function say what
//! those reach, so [`Listing::open`] reads the head and those classes and [`Listing::functions`]
//! then reads one function at a time. The whole text is held; no more than one function's
//! statements are.
//!
//! ⚠ **A listing is refused, never guessed at.** A name the runtime does not define, a call that
//! nothing declares or a shape the reader was not written for is an error with its line.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;

use super::ir::{
    BinOp, Callee, Const, Expr, Head, Kind, Returns, Scalar, Stmt, StoreKind, UnOp, Var, VarInfo,
    VmType, Width,
};
use super::tree::{self, Case, Label};

/// The deepest statements nest, and the deepest an expression does: every reader of the tree recurses.
// The deepest of 746 listings nests 45 blocks and 36 parentheses.
const MAX_NESTING: u32 = 256;

/// The targets one tuple assignment may have; the widest of 746 listings has 43.
const MAX_TUPLE: usize = 1024;

/// Why a listing was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ListingError {
    #[error("line {line}: {what}")]
    Syntax { line: u32, what: String },
    #[error("line {line}: `{path}` is called and nothing declares it")]
    Undeclared { line: u32, path: String },
}

impl ListingError {
    /// The line of the listing the error is on, counted from 1.
    pub fn line(&self) -> u32 {
        match self {
            ListingError::Syntax { line, .. } | ListingError::Undeclared { line, .. } => *line,
        }
    }
}

/// `// import N: program_0xHASH, types [a, b)`: a program this one calls into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub index: u32,
    pub program: u32,
    /// The run of type indices the import's types take.
    pub types: Range<u32>,
}

/// What a declaration's wrapper stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Call(Callee),
    /// `Vm.NewArray(type, p0)`, through the `Operations` member named.
    NewArray {
        ty: VmType,
        wrapper: String,
    },
}

/// One member of the declaration classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrapper {
    pub target: Target,
    /// The cells the call yields.
    pub returns: u32,
    pub params: u32,
}

/// The declaration classes a listing ends with, by path: `Native.NS.NAME`, `Imports.shared.func_X`.
#[derive(Clone, Debug, Default)]
pub struct Declarations {
    wrappers: HashMap<String, Wrapper>,
}

impl Declarations {
    pub fn get(&self, path: &str) -> Option<&Wrapper> {
        self.wrappers.get(path)
    }

    pub fn len(&self) -> usize {
        self.wrappers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wrappers.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Wrapper)> {
        self.wrappers.iter().map(|(path, w)| (path.as_str(), w))
    }
}

/// A listing with its head and declarations read, and its functions still text.
#[derive(Clone, Debug)]
pub struct Listing<'a> {
    /// The hash `[VmProgram(..)]` states.
    pub program: u32,
    pub imports: Vec<Import>,
    declarations: Declarations,
    functions: &'a str,
    /// The line `functions` starts on.
    first_line: u32,
    /// Whether `functions` ends with the brace that closes the program class, there being no
    /// declaration classes between the last function and it.
    closes: bool,
}

impl<'a> Listing<'a> {
    /// Reads a listing's head and its declaration classes.
    pub fn open(text: &'a str) -> Result<Self, ListingError> {
        let text = text.trim_start_matches('\u{feff}');
        let mut imports = Vec::new();
        let mut program = None;
        let mut pos = 0;
        let mut line = 0;
        let mut opened = false;
        for raw in text.split_inclusive('\n') {
            line += 1;
            pos += raw.len();
            let l = raw.trim();
            let refused = |what: &str| ListingError::Syntax {
                line,
                what: what.to_owned(),
            };
            if let Some(rest) = l.strip_prefix("// import ") {
                imports.push(import(rest).ok_or_else(|| refused("a malformed import line"))?);
            } else if let Some(rest) = l.strip_prefix("[VmProgram(") {
                let hash = rest
                    .strip_prefix("0x")
                    .and_then(|r| r.strip_suffix("U)]"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok());
                program = Some(hash.ok_or_else(|| refused("a malformed `[VmProgram(..)]` line"))?);
            } else if l == "{" {
                opened = true;
                break;
            } else if !(l.is_empty()
                || l.starts_with("//")
                || l.starts_with('#')
                || l.starts_with("using ")
                || l.starts_with("namespace ")
                || l.starts_with("public static class Program_"))
            {
                return Err(refused("a line the head of a listing does not hold"));
            }
        }
        let program = match program {
            Some(program) if opened => program,
            _ => {
                return Err(ListingError::Syntax {
                    line,
                    what: "the listing has no `[VmProgram(..)]` line and class".to_owned(),
                });
            }
        };
        // The program class opens with a `GuidEmpty` property, which no function needs read.
        while pos < text.len() {
            let rest = &text[pos..];
            let end = rest.find('\n').map_or(rest.len(), |n| n + 1);
            let l = rest[..end].trim();
            if !(l.is_empty() || l.starts_with("private static VmValues GuidEmpty")) {
                break;
            }
            pos += end;
            line += 1;
        }
        let body = &text[pos..];
        let first_line = line + 1;
        let (functions, declarations) = match body.find("public static class ") {
            Some(at) => {
                let functions = &body[..at];
                let lines = functions.bytes().filter(|b| *b == b'\n').count();
                let mut parser = Parser::new(&body[at..], first_line + lines as u32, None);
                parser.bump()?;
                (functions, parser.declarations()?)
            }
            None => (body, Declarations::default()),
        };
        Ok(Listing {
            program,
            imports,
            declarations,
            functions,
            first_line,
            closes: functions.len() == body.len(),
        })
    }

    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }

    /// The functions in the order the listing holds them, each read when it is asked for.
    pub fn functions(&self) -> Functions<'_, 'a> {
        let mut parser = Parser::new(self.functions, self.first_line, Some(&self.declarations));
        parser.program = self.program;
        Functions {
            parser,
            closes: self.closes,
            primed: false,
            done: false,
        }
    }
}

fn import(rest: &str) -> Option<Import> {
    let (index, rest) = rest.split_once(": program_0x")?;
    let (program, rest) = rest.split_once(", types [")?;
    let (first, rest) = rest.split_once(", ")?;
    let end = rest.strip_suffix(')')?;
    Some(Import {
        index: index.parse().ok()?,
        program: u32::from_str_radix(program, 16).ok()?,
        types: first.parse().ok()?..end.parse().ok()?,
    })
}

/// A listing's functions, read one at a time; the first error ends the iteration.
pub struct Functions<'d, 'a> {
    parser: Parser<'a, 'd>,
    /// Whether the brace that closes the program class follows the last function.
    closes: bool,
    primed: bool,
    done: bool,
}

impl Functions<'_, '_> {
    fn step(&mut self) -> Result<Option<tree::Function>, ListingError> {
        if !self.primed {
            self.parser.bump()?;
            self.primed = true;
        }
        // A brace anywhere else is one too many, and would hide every function after it.
        if self.closes && self.parser.eat("}")? {
            return match self.parser.tok {
                Tok::End => Ok(None),
                _ => Err(self.parser.unexpected("the end of the listing")),
            };
        }
        match self.parser.tok {
            Tok::End if !self.closes => Ok(None),
            _ => self.parser.function().map(Some),
        }
    }
}

impl Iterator for Functions<'_, '_> {
    type Item = Result<tree::Function, ListingError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(f)) => Some(Ok(f)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    /// An integer literal's value and the C# type it has.
    Int(u64, Kind),
    F32(f32),
    F64(f64),
    Str(String),
    Punct(&'static str),
    End,
}

impl Tok<'_> {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(word) => format!("`{word}`"),
            Tok::Int(..) | Tok::F32(_) | Tok::F64(_) => "a number".to_owned(),
            Tok::Str(_) => "a string".to_owned(),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::End => "the end of the text".to_owned(),
        }
    }
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// The line `pos` is on, counted from 1 over the whole listing.
    line: u32,
}

impl<'a> Lexer<'a> {
    fn refused(&self, what: &str) -> ListingError {
        ListingError::Syntax {
            line: self.line,
            what: what.to_owned(),
        }
    }

    /// Skips blanks and line ends, and `//` comments too when `comments`.
    fn skip_blank(&mut self, comments: bool) {
        let src = self.src;
        let bytes = src.as_bytes();
        while let Some(b) = bytes.get(self.pos).copied() {
            match b {
                b'\n' => self.line += 1,
                b' ' | b'\t' | b'\r' => {}
                b'/' if comments && bytes.get(self.pos + 1) == Some(&b'/') => {
                    let rest = &src[self.pos..];
                    self.pos += rest.find('\n').unwrap_or(rest.len());
                    continue;
                }
                _ => break,
            }
            self.pos += 1;
        }
    }

    /// The reason an `// INCOMPLETE:` comment gives, when one comes next.
    fn incomplete(&mut self) -> Option<&'a str> {
        self.skip_blank(false);
        let src = self.src;
        let rest = src[self.pos..].strip_prefix("// INCOMPLETE:")?;
        let reason = &rest[..rest.find('\n').unwrap_or(rest.len())];
        self.pos = src.len() - rest.len() + reason.len();
        Some(reason.trim())
    }

    fn token(&mut self) -> Result<Tok<'a>, ListingError> {
        self.skip_blank(true);
        let src = self.src;
        let bytes = src.as_bytes();
        let Some(c) = bytes.get(self.pos).copied() else {
            return Ok(Tok::End);
        };
        let start = self.pos;
        if c.is_ascii_alphabetic() || c == b'_' {
            while bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                self.pos += 1;
            }
            return Ok(Tok::Ident(&src[start..self.pos]));
        }
        if c.is_ascii_digit() {
            return self.number();
        }
        if c == b'"' {
            return self.string();
        }
        let punct = match (c, bytes.get(self.pos + 1).copied()) {
            (b'<', Some(b'<')) => "<<",
            (b'>', Some(b'>')) => ">>",
            (b'=', Some(b'=')) => "==",
            (b'!', Some(b'=')) => "!=",
            (b'<', Some(b'=')) => "<=",
            (b'>', Some(b'=')) => ">=",
            (b'&', Some(b'&')) => "&&",
            (b'|', Some(b'|')) => "||",
            (b'+', Some(b'+')) => "++",
            (b'-', Some(b'-')) => "--",
            (b'=', Some(b'>')) => "=>",
            (b':', Some(b':')) => "::",
            (b'(', _) => "(",
            (b')', _) => ")",
            (b'[', _) => "[",
            (b']', _) => "]",
            (b'{', _) => "{",
            (b'}', _) => "}",
            (b',', _) => ",",
            (b';', _) => ";",
            (b':', _) => ":",
            (b'.', _) => ".",
            (b'?', _) => "?",
            (b'=', _) => "=",
            (b'<', _) => "<",
            (b'>', _) => ">",
            (b'+', _) => "+",
            (b'-', _) => "-",
            (b'*', _) => "*",
            (b'/', _) => "/",
            (b'%', _) => "%",
            (b'&', _) => "&",
            (b'|', _) => "|",
            (b'^', _) => "^",
            (b'~', _) => "~",
            (b'!', _) => "!",
            _ => return Err(self.refused("a character no token starts with")),
        };
        self.pos += punct.len();
        Ok(Tok::Punct(punct))
    }

    fn number(&mut self) -> Result<Tok<'a>, ListingError> {
        let src = self.src;
        let bytes = src.as_bytes();
        let start = self.pos;
        let digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
        let hex = bytes[start] == b'0' && matches!(bytes.get(start + 1), Some(b'x' | b'X'));
        let mut real = false;
        if hex {
            self.pos += 2;
            while bytes.get(self.pos).is_some_and(u8::is_ascii_hexdigit) {
                self.pos += 1;
            }
        } else {
            while digit(self.pos) {
                self.pos += 1;
            }
            if bytes.get(self.pos) == Some(&b'.') && digit(self.pos + 1) {
                real = true;
                self.pos += 1;
                while digit(self.pos) {
                    self.pos += 1;
                }
            }
            if matches!(bytes.get(self.pos), Some(b'e' | b'E')) {
                let signed = matches!(bytes.get(self.pos + 1), Some(b'+' | b'-'));
                let exponent = self.pos + 1 + usize::from(signed);
                if digit(exponent) {
                    real = true;
                    self.pos = exponent;
                    while digit(self.pos) {
                        self.pos += 1;
                    }
                }
            }
        }
        let digits = &src[start..self.pos];
        let suffix_start = self.pos;
        while bytes.get(self.pos).is_some_and(u8::is_ascii_alphabetic) {
            self.pos += 1;
        }
        let suffix = src[suffix_start..self.pos].to_ascii_uppercase();
        let malformed = || self.refused("a malformed number");
        let integer = matches!(suffix.as_str(), "" | "U" | "L" | "UL" | "LU") && !real;
        if !integer {
            return match suffix.as_str() {
                "F" if !hex => digits.parse().map(Tok::F32).map_err(|_| malformed()),
                "D" | "" if !hex => digits.parse().map(Tok::F64).map_err(|_| malformed()),
                _ => Err(malformed()),
            };
        }
        let value = match hex {
            true => u64::from_str_radix(&digits[2..], 16),
            false => digits.parse::<u64>(),
        };
        let Ok(value) = value else {
            return Err(malformed());
        };
        // C# gives a literal the first of `int`, `uint`, `long` and `ulong` that its value fits
        // and its suffix allows.
        let kind = match suffix.as_str() {
            "" if value <= i32::MAX as u64 => Kind::I32,
            "" | "U" if value <= u32::MAX as u64 => Kind::U32,
            "" | "L" if value <= i64::MAX as u64 => Kind::I64,
            _ => Kind::U64,
        };
        Ok(Tok::Int(value, kind))
    }

    fn string(&mut self) -> Result<Tok<'a>, ListingError> {
        let src = self.src;
        let bytes = src.as_bytes();
        self.pos += 1;
        let mut out = String::new();
        let mut run = self.pos;
        loop {
            match bytes.get(self.pos).copied() {
                None | Some(b'\n') => return Err(self.refused("a string with no closing quote")),
                Some(b'"') => {
                    out.push_str(&src[run..self.pos]);
                    self.pos += 1;
                    return Ok(Tok::Str(out));
                }
                Some(b'\\') => {
                    out.push_str(&src[run..self.pos]);
                    let escape = bytes.get(self.pos + 1).copied();
                    self.pos += 2;
                    match escape {
                        Some(b'\\') => out.push('\\'),
                        Some(b'"') => out.push('"'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'0') => out.push('\0'),
                        Some(b'u') => {
                            let code = src
                                .get(self.pos..self.pos + 4)
                                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                                .and_then(char::from_u32);
                            let Some(code) = code else {
                                return Err(self.refused("a malformed `\\u` escape"));
                            };
                            out.push(code);
                            self.pos += 4;
                        }
                        _ => return Err(self.refused("a string escape the reader does not know")),
                    }
                    run = self.pos;
                }
                Some(_) => self.pos += 1,
            }
        }
    }
}

struct Parser<'a, 'd> {
    lexer: Lexer<'a>,
    tok: Tok<'a>,
    /// The line `tok` is on.
    line: u32,
    /// `None` while the declarations themselves are read.
    decls: Option<&'d Declarations>,
    program: u32,
    /// The statements and expressions open around the token.
    depth: u32,
    /// The offsets of the functions read.
    offsets: HashSet<u32>,
}

impl<'a, 'd> Parser<'a, 'd> {
    /// A parser over `src`, whose first line is `line`; the first `bump` reads the first token.
    fn new(src: &'a str, line: u32, decls: Option<&'d Declarations>) -> Self {
        Parser {
            lexer: Lexer { src, pos: 0, line },
            tok: Tok::End,
            line,
            decls,
            program: 0,
            depth: 0,
            offsets: HashSet::new(),
        }
    }

    /// Reads what nests one level deeper, refused past [`MAX_NESTING`].
    fn nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, ListingError>,
    ) -> Result<T, ListingError> {
        if self.depth >= MAX_NESTING {
            return Err(self.refused(format!("nested more than {MAX_NESTING} deep")));
        }
        self.depth += 1;
        let read = read(self);
        self.depth -= 1;
        read
    }

    fn refused(&self, what: impl Into<String>) -> ListingError {
        ListingError::Syntax {
            line: self.line,
            what: what.into(),
        }
    }

    fn unexpected(&self, wanted: &str) -> ListingError {
        self.refused(format!("expected {wanted}, found {}", self.tok.describe()))
    }

    /// Moves to the next token and hands back the one left.
    fn bump(&mut self) -> Result<Tok<'a>, ListingError> {
        let next = self.lexer.token()?;
        self.line = self.lexer.line;
        Ok(std::mem::replace(&mut self.tok, next))
    }

    fn at(&self, punct: &str) -> bool {
        matches!(&self.tok, Tok::Punct(p) if *p == punct)
    }

    fn at_word(&self, word: &str) -> bool {
        matches!(&self.tok, Tok::Ident(w) if *w == word)
    }

    fn eat(&mut self, punct: &str) -> Result<bool, ListingError> {
        let found = self.at(punct);
        if found {
            self.bump()?;
        }
        Ok(found)
    }

    fn expect(&mut self, punct: &str) -> Result<(), ListingError> {
        match self.eat(punct)? {
            true => Ok(()),
            false => Err(self.unexpected(&format!("`{punct}`"))),
        }
    }

    fn word(&mut self, word: &str) -> Result<(), ListingError> {
        if !self.at_word(word) {
            return Err(self.unexpected(&format!("`{word}`")));
        }
        self.bump()?;
        Ok(())
    }

    fn words(&mut self, words: &[&str]) -> Result<(), ListingError> {
        words.iter().try_for_each(|word| self.word(word))
    }

    fn ident(&mut self) -> Result<&'a str, ListingError> {
        match self.tok {
            Tok::Ident(word) => {
                self.bump()?;
                Ok(word)
            }
            _ => Err(self.unexpected("a name")),
        }
    }

    fn int(&mut self) -> Result<u64, ListingError> {
        match self.tok {
            Tok::Int(value, _) => {
                self.bump()?;
                Ok(value)
            }
            _ => Err(self.unexpected("an integer")),
        }
    }

    fn u32(&mut self) -> Result<u32, ListingError> {
        let line = self.line;
        u32::try_from(self.int()?).map_err(|_| ListingError::Syntax {
            line,
            what: "an integer too large for the place it is in".to_owned(),
        })
    }

    fn string(&mut self) -> Result<String, ListingError> {
        if !matches!(self.tok, Tok::Str(_)) {
            return Err(self.unexpected("a string"));
        }
        match self.bump()? {
            Tok::Str(s) => Ok(s),
            _ => Err(self.refused("expected a string")),
        }
    }

    /// `, true` or nothing, before the `)` of a call that takes a statics flag.
    fn statics_flag(&mut self) -> Result<bool, ListingError> {
        if !self.eat(",")? {
            return Ok(false);
        }
        match self.ident()? {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(self.refused("expected `true` or `false`")),
        }
    }

    /// `new VmType("name")` or `new VmType("name", byteSize, id, flags)`.
    fn vm_type(&mut self) -> Result<VmType, ListingError> {
        self.words(&["new", "VmType"])?;
        self.expect("(")?;
        let name = self.string()?;
        let (mut byte_size, mut id, mut flags) = (0, 0, 0);
        if self.eat(",")? {
            byte_size = self.u32()?;
            self.expect(",")?;
            id = self.u32()?;
            self.expect(",")?;
            flags = self.u32()?;
        }
        self.expect(")")?;
        Ok(VmType {
            name,
            byte_size,
            id,
            flags,
        })
    }

    /// The declaration classes, up to the brace that closes the program class.
    fn declarations(&mut self) -> Result<Declarations, ListingError> {
        let mut decls = Declarations::default();
        let mut classes: Vec<&'a str> = Vec::new();
        loop {
            if self.eat("}")? {
                if classes.pop().is_none() {
                    break;
                }
                continue;
            }
            self.words(&["public", "static"])?;
            if self.at_word("class") {
                self.bump()?;
                classes.push(self.ident()?);
                self.expect("{")?;
                continue;
            }
            let line = self.line;
            let ty = self.ident()?;
            let name = self.ident()?;
            self.expect("(")?;
            let mut params = 0;
            while !self.at(")") {
                if params > 0 {
                    self.expect(",")?;
                }
                self.word("VmValue")?;
                if self.ident()? != format!("p{params}") {
                    return Err(self.refused("a wrapper whose parameters are not p0, p1, .."));
                }
                params += 1;
            }
            self.expect(")")?;
            self.expect("=>")?;
            self.word("Vm")?;
            self.expect(".")?;
            let call = self.ident()?;
            self.expect("(")?;
            let (target, returns) = self.wrapped(&classes, name, call)?;
            if matches!(target, Target::NewArray { .. }) && params != 1 {
                return Err(self.refused("an array wrapper that does not take the length alone"));
            }
            for n in 0..params {
                self.expect(",")?;
                if self.ident()? != format!("p{n}") {
                    return Err(
                        self.refused("a wrapper that does not pass its parameters in order")
                    );
                }
            }
            self.expect(")")?;
            let mut cell = matches!(target, Target::NewArray { .. });
            if self.eat("[")? {
                cell = self.int()? == 0;
                self.expect("]")?;
            }
            self.expect(";")?;
            let agrees = match ty {
                "void" => returns == 0 && !cell,
                "VmValue" => returns == 1 && cell,
                "VmValues" => returns >= 2 && !cell,
                _ => false,
            };
            if !agrees {
                return Err(ListingError::Syntax {
                    line,
                    what: format!("`{name}` is declared `{ty}` and yields {returns} cell(s)"),
                });
            }
            let path = format!("{}.{name}", classes.join("."));
            let wrapper = Wrapper {
                target,
                returns,
                params,
            };
            if decls.wrappers.insert(path, wrapper).is_some() {
                return Err(ListingError::Syntax {
                    line,
                    what: format!("`{name}` is declared twice"),
                });
            }
        }
        match self.tok {
            Tok::End => Ok(decls),
            _ => Err(self.unexpected("the end of the listing")),
        }
    }

    /// What `Vm.<call>(` stands for in a member `name` of `classes`, read up to its own arguments.
    fn wrapped(
        &mut self,
        classes: &[&str],
        name: &str,
        call: &str,
    ) -> Result<(Target, u32), ListingError> {
        let callee = match (classes, call) {
            (["Imports", module], "Script") => {
                let program = self.u32()?;
                self.expect(",")?;
                let at = self.u32()?;
                Callee::Import {
                    module: (*module).to_owned(),
                    name: name.to_owned(),
                    program,
                    at,
                }
            }
            (["Native", namespace @ ..], "Native") if namespace.len() <= 1 => {
                let hash = self.int()?;
                self.expect(",")?;
                let rva = self.u32()?;
                Callee::Native {
                    namespace: namespace.first().map(|ns| (*ns).to_owned()),
                    name: name.to_owned(),
                    hash,
                    rva,
                }
            }
            (["Builtins", path @ ..], "Builtin") => {
                let ty = self.u32()?;
                self.expect(",")?;
                let method = self.u32()?;
                let mut path = path.join(".");
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(name);
                Callee::Builtin { path, ty, method }
            }
            (["Virtual", class], "Virtual") => {
                let ty = self.string()?;
                self.expect(",")?;
                let slot = self.u32()?;
                Callee::Virtual {
                    class: (*class).to_owned(),
                    method: name.to_owned(),
                    ty,
                    slot,
                }
            }
            (["Operations"], "Intrinsic") => Callee::Intrinsic {
                name: self.string()?,
                ty: None,
                wrapper: Some(name.to_owned()),
            },
            (["Operations"], "NewArray") => {
                let target = Target::NewArray {
                    ty: self.vm_type()?,
                    wrapper: name.to_owned(),
                };
                return Ok((target, 1));
            }
            _ => {
                let class = classes.join(".");
                return Err(self.refused(format!("`Vm.{call}` in a member of `{class}`")));
            }
        };
        self.expect(",")?;
        Ok((Target::Call(callee), self.u32()?))
    }

    /// `[VmFunction(0xOFFSET)]` and the method under it.
    fn function(&mut self) -> Result<tree::Function, ListingError> {
        let first = self.line;
        self.expect("[")?;
        self.word("VmFunction")?;
        self.expect("(")?;
        let at = self.u32()?;
        if !self.offsets.insert(at) {
            return Err(self.refused(format!("a second function at offset {at:06X}")));
        }
        self.expect(")")?;
        self.expect("]")?;
        self.words(&["public", "static"])?;
        let returns = match self.ident()? {
            "void" => Returns::Void,
            "VmValue" => Returns::Cell,
            "VmValues" => Returns::Cells,
            _ => {
                return Err(self.refused("a function that returns none of void, VmValue, VmValues"))
            }
        };
        let name = self.ident()?.to_owned();
        self.expect("(")?;
        let mut params = 0;
        while !self.at(")") {
            if params > 0 {
                self.expect(",")?;
            }
            self.word("VmValue")?;
            if self.ident()? != format!("arg{params}") {
                return Err(self.refused("a function whose arguments are not arg0, arg1, .."));
            }
            params += 1;
        }
        self.expect(")")?;
        if !self.at("{") {
            return Err(self.unexpected("`{`"));
        }
        let mut head = Head {
            program: self.program,
            at,
            name,
            returns,
            params,
            locals: 0,
            vars: Default::default(),
            incomplete: None,
        };
        // The comment that says a body is missing comes before any token of it.
        let incomplete = self.lexer.incomplete().map(str::to_owned);
        self.bump()?;
        if incomplete.is_some() {
            head.incomplete = incomplete;
            self.words(&["throw", "new", "global"])?;
            self.expect("::")?;
            self.word("System")?;
            self.expect(".")?;
            self.word("NotSupportedException")?;
            self.expect("(")?;
            self.string()?;
            self.expect(")")?;
            self.expect(";")?;
            self.expect("}")?;
            return Ok(tree::Function {
                head,
                body: Vec::new(),
            });
        }
        self.preamble(&mut head)?;
        let mut body = Vec::new();
        while !self.at("}") {
            self.statement(&mut body)?;
        }
        self.bump()?;
        if let Some(name) = undeclared(&head, &body) {
            let what = format!("`{name}` is named and the function declares none");
            return Err(ListingError::Syntax { line: first, what });
        }
        Ok(tree::Function { head, body })
    }

    /// The frames, the names of their slots and the temporaries a method declares before its code.
    fn preamble(&mut self, head: &mut Head) -> Result<(), ListingError> {
        self.word("VmValue")?;
        self.expect("[")?;
        self.expect("]")?;
        self.word("parameters")?;
        self.expect("=")?;
        self.words(&["new", "VmValue"])?;
        self.expect("[")?;
        self.expect("]")?;
        self.expect("{")?;
        let mut listed = 0;
        while !self.at("}") {
            if listed > 0 {
                self.expect(",")?;
            }
            self.ident()?;
            listed += 1;
        }
        self.bump()?;
        self.expect(";")?;
        if listed != head.params {
            return Err(self.refused("a parameter frame that does not hold every argument"));
        }
        self.word("VmValue")?;
        self.expect("[")?;
        self.expect("]")?;
        self.word("locals")?;
        self.expect("=")?;
        self.words(&["new", "VmValue"])?;
        self.expect("[")?;
        head.locals = self.u32()?;
        self.expect("]")?;
        self.expect(";")?;
        while self.at_word("ref") {
            self.bump()?;
            self.word("VmValue")?;
            let name = self.ident()?;
            self.expect("=")?;
            self.word("ref")?;
            let frame = self.ident()?;
            self.expect("[")?;
            let index = self.u32()?;
            self.expect("]")?;
            self.expect(";")?;
            let slot = match frame {
                "parameters" => Some(Var::Param(index)),
                "locals" => Some(Var::Local(index)),
                _ => None,
            };
            if slot.is_none() || Var::from_name(name) != slot {
                return Err(self.refused(format!("`{name}` does not name `{frame}[{index}]`")));
            }
        }
        loop {
            let width = if self.at_word("VmValue") {
                Width::Cell
            } else if self.at_word("VmValues") {
                Width::Cells
            } else {
                return Ok(());
            };
            self.bump()?;
            let name = self.ident()?;
            self.expect("=")?;
            match width {
                Width::Cell => self.word("default")?,
                Width::Cells => {
                    self.word("null")?;
                    self.expect("!")?;
                }
            }
            self.expect(";")?;
            let var = Var::from_name(name).filter(|v| v.is_temporary());
            let Some(var) = var else {
                return Err(self.refused(format!(
                    "`{name}` is declared and is no `v_` or `stack_` name"
                )));
            };
            if head.vars.insert(var, VarInfo::new(width)).is_some() {
                return Err(self.refused(format!("`{name}` is declared twice")));
            }
        }
    }

    fn block(&mut self) -> Result<Vec<tree::Stmt>, ListingError> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.at("}") {
            self.statement(&mut body)?;
        }
        self.bump()?;
        Ok(body)
    }

    fn label(&mut self) -> Result<Label, ListingError> {
        let name = self.ident()?;
        Label::from_name(name).ok_or_else(|| self.refused(format!("`{name}` is not a label")))
    }

    /// Reads one statement into `out`; `v = p++;` alone becomes two.
    fn statement(&mut self, out: &mut Vec<tree::Stmt>) -> Result<(), ListingError> {
        self.nested(|parser| parser.one_statement(out))
    }

    fn one_statement(&mut self, out: &mut Vec<tree::Stmt>) -> Result<(), ListingError> {
        let word = match self.tok {
            Tok::Ident(word) => word,
            _ => "",
        };
        match word {
            "if" => {
                self.bump()?;
                self.expect("(")?;
                let cond = self.expr()?;
                self.expect(")")?;
                let then = self.block()?;
                let mut els = Vec::new();
                if self.at_word("else") {
                    self.bump()?;
                    if self.at_word("if") {
                        self.statement(&mut els)?;
                    } else {
                        els = self.block()?;
                    }
                }
                out.push(tree::Stmt::If { cond, then, els });
            }
            "while" => {
                self.bump()?;
                self.expect("(")?;
                let cond = self.expr()?;
                self.expect(")")?;
                let body = self.block()?;
                out.push(tree::Stmt::While { cond, body });
            }
            "switch" => {
                self.bump()?;
                self.expect("(")?;
                let value = self.expr()?;
                self.expect(")")?;
                self.expect("{")?;
                let mut cases = Vec::new();
                let mut labels = BTreeSet::new();
                while !self.at("}") {
                    let case = self.case()?;
                    if !case.labels.iter().all(|label| labels.insert(*label)) {
                        return Err(self.refused("a case stated twice in one switch"));
                    }
                    cases.push(case);
                }
                self.bump()?;
                out.push(tree::Stmt::Switch { value, cases });
            }
            "goto" => {
                self.bump()?;
                let label = self.label()?;
                self.expect(";")?;
                out.push(tree::Stmt::Goto(label));
            }
            "return" => {
                self.bump()?;
                let value = match self.at(";") {
                    true => None,
                    false => Some(self.expr()?),
                };
                self.expect(";")?;
                out.push(tree::Stmt::Return(value));
            }
            "break" => {
                self.bump()?;
                self.expect(";")?;
                out.push(tree::Stmt::Break);
            }
            "continue" => {
                self.bump()?;
                self.expect(";")?;
                out.push(tree::Stmt::Continue);
            }
            "throw" => {
                self.words(&["throw", "new", "global"])?;
                self.expect("::")?;
                self.word("System")?;
                self.expect(".")?;
                self.word("InvalidOperationException")?;
                self.expect("(")?;
                self.string()?;
                self.expect(")")?;
                self.expect(";")?;
                out.push(tree::Stmt::Unreachable);
            }
            "zero_bytes" => {
                self.bump()?;
                self.expect("(")?;
                let addr = self.expr()?;
                self.expect(",")?;
                let bytes = self.u32()?;
                self.expect(")")?;
                self.expect(";")?;
                out.push(tree::Stmt::Simple(Stmt::ZeroBytes { addr, bytes }));
            }
            "increment_i32" | "decrement_i32" => {
                self.bump()?;
                self.expect("(")?;
                let addr = self.expr()?;
                self.expect(",")?;
                let offset = self.u32()?;
                self.expect(")")?;
                self.expect(";")?;
                out.push(tree::Stmt::Simple(Stmt::StepI32 {
                    addr,
                    offset,
                    down: word == "decrement_i32",
                }));
            }
            "store_bytes" => {
                self.bump()?;
                self.expect("(")?;
                let addr = self.expr()?;
                self.expect(",")?;
                let offset = self.u32()?;
                self.expect(",")?;
                let bytes = self.u32()?;
                self.expect(",")?;
                let value = self.expr()?;
                self.expect(")")?;
                self.expect(";")?;
                out.push(tree::Stmt::Simple(Stmt::StoreBytes {
                    addr,
                    offset,
                    bytes,
                    value,
                }));
            }
            _ if word.starts_with("store_") => {
                let kind = word.strip_prefix("store_").and_then(StoreKind::from_name);
                if kind.is_none() && word != "store_f32_slots" {
                    return Err(self.refused(format!("`{word}` is no store the runtime defines")));
                }
                self.bump()?;
                self.expect("(")?;
                let addr = self.expr()?;
                self.expect(",")?;
                let offset = self.u32()?;
                self.expect(",")?;
                let value = self.expr()?;
                self.expect(")")?;
                self.expect(";")?;
                out.push(tree::Stmt::Simple(match kind {
                    Some(kind) => Stmt::Store {
                        kind,
                        addr,
                        offset,
                        value,
                    },
                    None => Stmt::StoreF32Slots {
                        addr,
                        offset,
                        value,
                    },
                }));
            }
            _ if Label::from_name(word).is_some() => {
                let label = self.label()?;
                self.expect(":")?;
                self.expect(";")?;
                out.push(tree::Stmt::Label(label));
            }
            _ if self.at("(") => {
                let line = self.line;
                self.bump()?;
                let mut targets = vec![self.variable()?];
                while self.eat(",")? {
                    if targets.len() >= MAX_TUPLE {
                        return Err(self.refused(format!("a tuple of more than {MAX_TUPLE}")));
                    }
                    targets.push(self.variable()?);
                }
                self.expect(")")?;
                self.expect("=")?;
                self.expect("(")?;
                let mut values = vec![self.expr()?];
                while self.eat(",")? {
                    values.push(self.expr()?);
                }
                self.expect(")")?;
                self.expect(";")?;
                if targets.len() != values.len() {
                    return Err(ListingError::Syntax {
                        line,
                        what: "a tuple assignment of unequal sides".to_owned(),
                    });
                }
                out.push(tree::Stmt::Simple(Stmt::AssignMany { targets, values }));
            }
            _ => self.assignment_or_call(out)?,
        }
        Ok(())
    }

    /// `case N:` and `default:` labels, then the statements up to the next of them.
    fn case(&mut self) -> Result<Case, ListingError> {
        let mut labels = Vec::new();
        loop {
            if self.at_word("case") {
                self.bump()?;
                let negative = self.eat("-")?;
                let value = i64::try_from(self.int()?);
                let value = value.map_err(|_| self.refused("a case label too large"))?;
                labels.push(Some(if negative { -value } else { value }));
            } else if self.at_word("default") {
                self.bump()?;
                labels.push(None);
            } else {
                break;
            }
            self.expect(":")?;
        }
        if labels.is_empty() {
            return Err(self.unexpected("`case` or `default`"));
        }
        let mut body = Vec::new();
        while !(self.at("}") || self.at_word("case") || self.at_word("default")) {
            self.statement(&mut body)?;
        }
        Ok(Case { labels, body })
    }

    /// A name that is a variable.
    fn variable(&mut self) -> Result<Var, ListingError> {
        let name = self.ident()?;
        Var::from_name(name).ok_or_else(|| self.refused(format!("`{name}` is not a variable")))
    }

    fn assignment_or_call(&mut self, out: &mut Vec<tree::Stmt>) -> Result<(), ListingError> {
        let line = self.line;
        let refused = |what: &str| ListingError::Syntax {
            line,
            what: what.to_owned(),
        };
        let left = self.postfix()?;
        if self.eat("=")? {
            let Expr::Var(target) = left else {
                return Err(refused("an assignment to something that is no variable"));
            };
            let value = self.expr()?;
            // `v = p++;` reads `p` and then steps it; nothing else in a listing steps inside an expression.
            if let Some(down) = self.step()? {
                self.expect(";")?;
                return match value {
                    Expr::Var(source) if source != target => {
                        let value = Expr::Var(source);
                        out.push(tree::Stmt::Simple(Stmt::Assign { target, value }));
                        out.push(tree::Stmt::Simple(stepped(source, down)));
                        Ok(())
                    }
                    _ => Err(refused(
                        "a `++` or `--` on something that is no other variable",
                    )),
                };
            }
            self.expect(";")?;
            out.push(tree::Stmt::Simple(Stmt::Assign { target, value }));
        } else if let Some(down) = self.step()? {
            self.expect(";")?;
            let Expr::Var(var) = left else {
                return Err(refused("a `++` or `--` on something that is no variable"));
            };
            out.push(tree::Stmt::Simple(stepped(var, down)));
        } else {
            self.expect(";")?;
            if !matches!(left, Expr::Call { .. }) {
                return Err(refused(
                    "an expression that is no call, standing as a statement",
                ));
            }
            out.push(tree::Stmt::Simple(Stmt::Eval(left)));
        }
        Ok(())
    }

    /// `++` or `--` when one comes next: whether it steps down.
    fn step(&mut self) -> Result<Option<bool>, ListingError> {
        if self.eat("++")? {
            return Ok(Some(false));
        }
        Ok(self.eat("--")?.then_some(true))
    }

    fn expr(&mut self) -> Result<Expr, ListingError> {
        let cond = self.binary(BinOp::LogicalOr.precedence())?;
        if !self.eat("?")? {
            return Ok(cond);
        }
        let then = self.expr()?;
        self.expect(":")?;
        let els = self.expr()?;
        Ok(Expr::Select {
            cond: Box::new(cond),
            then: Box::new(then),
            els: Box::new(els),
        })
    }

    /// A run of binary operators that bind at least as tightly as `min`.
    fn binary(&mut self, min: u8) -> Result<Expr, ListingError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.tok {
                Tok::Punct(p) => BinOp::from_symbol(p),
                _ => None,
            };
            let Some(op) = op.filter(|op| op.precedence() >= min) else {
                return Ok(left);
            };
            self.bump()?;
            let right = self.binary(op.precedence() + 1)?;
            left = Expr::binary(op, left, right);
        }
    }

    // Every way an expression nests comes through here: a parenthesis, an argument, an operator in front.
    fn unary(&mut self) -> Result<Expr, ListingError> {
        self.nested(Self::one_unary)
    }

    fn one_unary(&mut self) -> Result<Expr, ListingError> {
        if self.eat("!")? {
            let e = Box::new(self.unary()?);
            return Ok(Expr::Unary { op: UnOp::Not, e });
        }
        if self.eat("~")? {
            let e = Box::new(self.unary()?);
            return Ok(Expr::Unary {
                op: UnOp::BitNot,
                e,
            });
        }
        if self.eat("-")? {
            return Ok(negated(self.unary()?));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, ListingError> {
        let mut e = self.primary()?;
        while self.eat("[")? {
            let index = self.u32()?;
            self.expect("]")?;
            e = Expr::Cell {
                base: Box::new(e),
                index,
            };
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ListingError> {
        let line = self.line;
        match self.bump()? {
            Tok::Int(bits, kind) => Ok(Expr::Const(Const { bits, kind })),
            Tok::F32(value) => Ok(Expr::Const(Const::f32(value))),
            Tok::F64(value) => Ok(Expr::Const(Const {
                bits: value.to_bits(),
                kind: Kind::F64,
            })),
            Tok::Punct("(") => self.parenthesized(),
            Tok::Ident(word) => self.named(word, line),
            other => Err(ListingError::Syntax {
                line,
                what: format!("expected an expression, found {}", other.describe()),
            }),
        }
    }

    /// What follows a `(`: an expression in parentheses, or one of the listing's two C# casts.
    fn parenthesized(&mut self) -> Result<Expr, ListingError> {
        let to_int = self.at_word("int");
        if to_int || self.at_word("VmValue") {
            self.bump()?;
            self.expect(")")?;
            let e = self.unary()?;
            // `(VmValue)` converts to a cell, which every expression here already is.
            if !to_int {
                return Ok(e);
            }
            // `(int)` of an integer keeps its low bits, as `as_i32` does; of a float it would not.
            if !e.listed_kind().is_integer() {
                return Err(self.refused("an `(int)` cast of something that is no integer"));
            }
            return Ok(Expr::cast(Scalar::I32, e));
        }
        let e = self.expr()?;
        self.expect(")")?;
        Ok(e)
    }

    fn args(&mut self) -> Result<Vec<Expr>, ListingError> {
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.at(")") {
            if !args.is_empty() {
                self.expect(",")?;
            }
            args.push(self.expr()?);
        }
        self.bump()?;
        Ok(args)
    }

    /// The one argument of a call that takes one.
    fn arg(&mut self) -> Result<Box<Expr>, ListingError> {
        self.expect("(")?;
        let e = self.expr()?;
        self.expect(")")?;
        Ok(Box::new(e))
    }

    /// An expression that starts with the name `word`, already read from `line`.
    fn named(&mut self, word: &'a str, line: u32) -> Result<Expr, ListingError> {
        let refused = |what: String| ListingError::Syntax { line, what };
        match word {
            "true" => Ok(Expr::Const(Const::bool(true))),
            "false" => Ok(Expr::Const(Const::bool(false))),
            "GuidEmpty" => Ok(Expr::GuidEmpty),
            "unchecked" => Ok(*self.arg()?),
            "Truth" => Ok(Expr::Truth(self.arg()?)),
            "new" => {
                self.word("VmValues")?;
                Ok(Expr::Cells(self.args()?))
            }
            "float_bits" => {
                self.expect("(")?;
                let bits = u64::from(self.u32()?);
                self.expect(")")?;
                Ok(Expr::Const(Const {
                    bits,
                    kind: Kind::F32,
                }))
            }
            "zero_slots" => {
                self.expect("(")?;
                let cells = self.u32()?;
                self.expect(")")?;
                Ok(Expr::ZeroSlots(cells))
            }
            "vector3" => match <[Expr; 3]>::try_from(self.args()?) {
                Ok(parts) => Ok(Expr::Vector3(Box::new(parts))),
                Err(_) => Err(refused("`vector3` takes three values".to_owned())),
            },
            "byte_offset" => {
                self.expect("(")?;
                let addr = Box::new(self.expr()?);
                self.expect(",")?;
                let offset = Box::new(self.expr()?);
                self.expect(")")?;
                Ok(Expr::ByteOffset { addr, offset })
            }
            "load_bytes" => {
                self.expect("(")?;
                let addr = Box::new(self.expr()?);
                self.expect(",")?;
                let offset = self.u32()?;
                self.expect(",")?;
                let bytes = self.u32()?;
                self.expect(")")?;
                Ok(Expr::LoadBytes {
                    addr,
                    offset,
                    bytes,
                })
            }
            "extract_bytes" => {
                self.expect("(")?;
                let value = Box::new(self.expr()?);
                self.expect(",")?;
                let offset = self.u32()?;
                self.expect(",")?;
                let bytes = self.u32()?;
                self.expect(")")?;
                Ok(Expr::ExtractBytes {
                    value,
                    offset,
                    bytes,
                })
            }
            "Vm" => self.vm(line),
            "Native" | "Imports" | "Builtins" | "Operations" | "Virtual" => {
                self.wrapper_call(word, line)
            }
            _ => {
                if let Some(to) = word.strip_prefix("as_").and_then(Scalar::from_name) {
                    return Ok(Expr::Cast { to, e: self.arg()? });
                }
                if let Some(ty) = word.strip_prefix("load_").and_then(Scalar::from_name) {
                    // The runtime reads no `u32`, `i64` or float; a listing that does is another's.
                    if matches!(ty, Scalar::U32 | Scalar::I64 | Scalar::F32 | Scalar::F64) {
                        return Err(refused(format!("`{word}` is no load the runtime defines")));
                    }
                    self.expect("(")?;
                    let addr = Box::new(self.expr()?);
                    self.expect(",")?;
                    let offset = self.u32()?;
                    self.expect(")")?;
                    return Ok(Expr::Load { ty, addr, offset });
                }
                if let Some(var) = Var::from_name(word) {
                    return Ok(Expr::Var(var));
                }
                let local = word.strip_prefix("func_").filter(|hex| !hex.is_empty());
                if let Some(at) = local.and_then(|hex| u32::from_str_radix(hex, 16).ok()) {
                    let callee = Callee::Local {
                        at,
                        name: word.to_owned(),
                    };
                    return Ok(Expr::Call {
                        callee,
                        args: self.args()?,
                        returns: None,
                    });
                }
                Err(refused(format!("`{word}` is no name the reader knows")))
            }
        }
    }

    /// What follows `Vm`: one of the runtime's own calls.
    fn vm(&mut self, line: u32) -> Result<Expr, ListingError> {
        let refused = |what: String| ListingError::Syntax { line, what };
        self.expect(".")?;
        let call = self.ident()?;
        self.expect("(")?;
        let e = match call {
            "Address" => {
                let frame = self.ident()?;
                self.expect(",")?;
                let index = self.u32()?;
                match (frame, Var::from_name(frame)) {
                    ("locals", _) => Expr::AddrOf {
                        var: Var::Local(index),
                        cell: 0,
                    },
                    ("parameters", _) => Expr::AddrOf {
                        var: Var::Param(index),
                        cell: 0,
                    },
                    (_, Some(var)) if var.is_temporary() => Expr::AddrOf { var, cell: index },
                    _ => return Err(refused(format!("`Vm.Address` of `{frame}`"))),
                }
            }
            "Globals" => {
                let program = self.u32()?;
                let statics = self.statics_flag()?;
                self.expect(")")?;
                self.expect("[")?;
                let offset = self.u32()?;
                self.expect("]")?;
                return Ok(Expr::Var(Var::Global {
                    program,
                    offset,
                    statics,
                }));
            }
            "GlobalAddress" => {
                let program = self.u32()?;
                self.expect(",")?;
                let offset = self.u32()?;
                let statics = self.statics_flag()?;
                let var = Var::Global {
                    program,
                    offset,
                    statics,
                };
                Expr::AddrOf { var, cell: 0 }
            }
            "Function" => {
                let program = self.u32()?;
                self.expect(",")?;
                let at = self.u32()?;
                Expr::FunctionRef { program, at }
            }
            "BuiltinFunction" => {
                let ty = self.u32()?;
                self.expect(",")?;
                let method = self.u32()?;
                Expr::BuiltinFunctionRef { ty, method }
            }
            "StringConstant" => {
                let program = self.u32()?;
                self.expect(",")?;
                let offset = self.u32()?;
                Expr::StringConstant { program, offset }
            }
            "NewObject" => Expr::NewObject(self.vm_type()?),
            "BuiltinGlobal" => Expr::BuiltinGlobal(self.u32()?),
            "VirtualFunction" => {
                let receiver = Box::new(self.expr()?);
                self.expect(",")?;
                let ty = self.vm_type()?;
                self.expect(",")?;
                let slot = self.u32()?;
                Expr::VirtualFunctionRef { receiver, ty, slot }
            }
            "Intrinsic" | "TypedIntrinsic" => {
                let name = self.string()?;
                self.expect(",")?;
                let (ty, returns) = match call {
                    "Intrinsic" => (None, self.u32()?),
                    _ => (Some(self.vm_type()?), 1),
                };
                let mut args = Vec::new();
                while self.eat(",")? {
                    args.push(self.expr()?);
                }
                self.expect(")")?;
                // One cell comes out of `Vm.Intrinsic` as a group of one, which `[0]` opens.
                if ty.is_none() && returns == 1 {
                    self.expect("[")?;
                    if self.int()? != 0 {
                        return Err(refused(
                            "a cell past the one an intrinsic yields".to_owned(),
                        ));
                    }
                    self.expect("]")?;
                }
                let callee = Callee::Intrinsic {
                    name,
                    ty,
                    wrapper: None,
                };
                return Ok(Expr::Call {
                    callee,
                    args,
                    returns: Some(returns),
                });
            }
            _ => return Err(refused(format!("`Vm.{call}` is no call the reader knows"))),
        };
        self.expect(")")?;
        Ok(e)
    }

    /// A call through the declaration classes, whose path starts with `root`.
    fn wrapper_call(&mut self, root: &str, line: u32) -> Result<Expr, ListingError> {
        let mut path = root.to_owned();
        while self.eat(".")? {
            path.push('.');
            path.push_str(self.ident()?);
        }
        let Some(wrapper) = self.decls.and_then(|decls| decls.get(&path)) else {
            return Err(ListingError::Undeclared { line, path });
        };
        let mut args = self.args()?;
        if args.len() != wrapper.params as usize {
            return Err(ListingError::Syntax {
                line,
                what: format!(
                    "`{path}` takes {} argument(s) and is given {}",
                    wrapper.params,
                    args.len()
                ),
            });
        }
        match (&wrapper.target, args.pop()) {
            (Target::NewArray { ty, wrapper }, Some(len)) => Ok(Expr::NewArray {
                ty: ty.clone(),
                len: Box::new(len),
                wrapper: wrapper.clone(),
            }),
            (Target::Call(callee), last) => {
                args.extend(last);
                Ok(Expr::Call {
                    callee: callee.clone(),
                    args,
                    returns: Some(wrapper.returns),
                })
            }
            (Target::NewArray { .. }, None) => Err(ListingError::Syntax {
                line,
                what: format!("`{path}` makes an array and is given no length"),
            }),
        }
    }
}

/// The first name a body uses that its function does not declare: a parameter or a local past those there are, a temporary no declaration names, a cell of a temporary of one cell.
fn undeclared(head: &Head, body: &[tree::Stmt]) -> Option<String> {
    let known = |var: Var| match var {
        Var::Param(n) => n < head.params,
        Var::Local(n) => n < head.locals,
        Var::Value { .. } | Var::Stack { .. } => head.vars.contains_key(&var),
        Var::Global { .. } => true,
    };
    let mut found = None;
    let mut note = |name: String| {
        found.get_or_insert(name);
    };
    tree::each_simple(body, &mut |stmt| {
        for target in stmt.targets().iter().filter(|target| !known(**target)) {
            note(target.to_string());
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::Var(var) | Expr::AddrOf { var, .. } if !known(*var) => note(var.to_string()),
            Expr::Cell { base, index } => {
                if let Expr::Var(var) = &**base {
                    let one = head.vars.get(var).is_some_and(|v| v.width == Width::Cell);
                    if one && *index > 0 {
                        note(format!("{var}[{index}]"));
                    }
                }
            }
            _ => {}
        });
    });
    found
}

/// `var++` or `var--`: the low 32 bits stepped by one and widened with their sign.
fn stepped(var: Var, down: bool) -> Stmt {
    let op = if down { BinOp::Sub } else { BinOp::Add };
    let old = Expr::cast(Scalar::I32, Expr::Var(var));
    Stmt::Assign {
        target: var,
        value: Expr::binary(op, old, Expr::Const(Const::i32(1))),
    }
}

/// `-e`, with a literal's sign taken into the literal as C# reads it.
fn negated(e: Expr) -> Expr {
    let folded = match &e {
        Expr::Const(c) => match c.kind {
            Kind::I32 => Some(Const::i32((c.bits as i32).wrapping_neg())),
            Kind::I64 => Some(Const {
                bits: (c.bits as i64).wrapping_neg() as u64,
                kind: Kind::I64,
            }),
            Kind::F32 => Some(Const {
                bits: c.bits ^ (1 << 31),
                kind: Kind::F32,
            }),
            Kind::F64 => Some(Const {
                bits: c.bits ^ (1 << 63),
                kind: Kind::F64,
            }),
            // `-2147483648` is an `int` and `-9223372036854775808` a `long`, though the digits alone are not.
            Kind::U32 if c.bits == 1 << 31 => Some(Const::i32(i32::MIN)),
            Kind::U64 if c.bits == 1 << 63 => Some(Const {
                bits: 1 << 63,
                kind: Kind::I64,
            }),
            _ => None,
        },
        _ => None,
    };
    match folded {
        Some(c) => Expr::Const(c),
        None => Expr::Unary {
            op: UnOp::Neg,
            e: Box::new(e),
        },
    }
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
