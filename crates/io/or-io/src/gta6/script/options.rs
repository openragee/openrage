//! What a decompile is asked for: the level of its output and the passes that run.
//!
//! Every pass and every reconstruction has a name here, so that one can be switched off from the
//! command line and a pass that changes what code does can be found by halving the set.

/// How far the output is taken from the bytecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// The front end's own form in a listing's syntax: no pass runs.
    Lifted,
    /// Functions of structured statements, with no source construct rebuilt.
    Structured,
    /// Classes with their members, and the source constructs a pass can prove.
    Source,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Lifted, Level::Structured, Level::Source];

    pub fn name(self) -> &'static str {
        match self {
            Level::Lifted => "lifted",
            Level::Structured => "structured",
            Level::Source => "source",
        }
    }

    pub fn from_name(name: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|level| level.name() == name)
    }
}

/// One pass or one reconstruction, by the name that switches it.
// ⚠ In the order of `ALL`: a pass's place in a `PassSet` is its number here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pass {
    Values,
    Escape,
    Forward,
    DeadLoads,
    Constants,
    Coalesce,
    Interproc,
    Aggregates,
    Types,
    Kinds,
    Signs,
    Structure,
    Gotos,
    Names,
    HashText,
    NativeParams,
    RoleNames,
    Fields,
    Classes,
    Receivers,
    Constructors,
    Properties,
    Operators,
    Await,
    Iterators,
    Wait,
    Lambdas,
    LocalFunctions,
    Delegates,
    Events,
    Foreach,
    Using,
    Lock,
    Try,
    For,
    SwitchOn,
    NullOps,
    Compound,
    Concat,
    Initialisers,
    RefOut,
    Nullable,
    Tuples,
}

impl Pass {
    pub const ALL: [Pass; 43] = [
        Pass::Values,
        Pass::Escape,
        Pass::Forward,
        Pass::DeadLoads,
        Pass::Constants,
        Pass::Coalesce,
        Pass::Interproc,
        Pass::Aggregates,
        Pass::Types,
        Pass::Kinds,
        Pass::Signs,
        Pass::Structure,
        Pass::Gotos,
        Pass::Names,
        Pass::HashText,
        Pass::NativeParams,
        Pass::RoleNames,
        Pass::Fields,
        Pass::Classes,
        Pass::Receivers,
        Pass::Constructors,
        Pass::Properties,
        Pass::Operators,
        Pass::Await,
        Pass::Iterators,
        Pass::Wait,
        Pass::Lambdas,
        Pass::LocalFunctions,
        Pass::Delegates,
        Pass::Events,
        Pass::Foreach,
        Pass::Using,
        Pass::Lock,
        Pass::Try,
        Pass::For,
        Pass::SwitchOn,
        Pass::NullOps,
        Pass::Compound,
        Pass::Concat,
        Pass::Initialisers,
        Pass::RefOut,
        Pass::Nullable,
        Pass::Tuples,
    ];

    /// The four passes `decompile` has always run.
    pub const BASELINE: [Pass; 4] = [Pass::Values, Pass::Types, Pass::Structure, Pass::Names];

    /// The name the command line switches the pass by.
    pub fn name(self) -> &'static str {
        match self {
            Pass::Values => "values",
            Pass::Escape => "escape",
            Pass::Forward => "forward",
            Pass::DeadLoads => "dead-loads",
            Pass::Constants => "constants",
            Pass::Coalesce => "coalesce",
            Pass::Interproc => "interproc",
            Pass::Aggregates => "aggregates",
            Pass::Types => "types",
            Pass::Kinds => "kinds",
            Pass::Signs => "signs",
            Pass::Structure => "structure",
            Pass::Gotos => "gotos",
            Pass::Names => "names",
            Pass::HashText => "hash-text",
            Pass::NativeParams => "native-params",
            Pass::RoleNames => "role-names",
            Pass::Fields => "fields",
            Pass::Classes => "classes",
            Pass::Receivers => "receivers",
            Pass::Constructors => "constructors",
            Pass::Properties => "properties",
            Pass::Operators => "operators",
            Pass::Await => "await",
            Pass::Iterators => "iterators",
            Pass::Wait => "wait",
            Pass::Lambdas => "lambdas",
            Pass::LocalFunctions => "local-functions",
            Pass::Delegates => "delegates",
            Pass::Events => "events",
            Pass::Foreach => "foreach",
            Pass::Using => "using",
            Pass::Lock => "lock",
            Pass::Try => "try",
            Pass::For => "for",
            Pass::SwitchOn => "switch-on",
            Pass::NullOps => "null-ops",
            Pass::Compound => "compound",
            Pass::Concat => "concat",
            Pass::Initialisers => "initialisers",
            Pass::RefOut => "ref-out",
            Pass::Nullable => "nullable",
            Pass::Tuples => "tuples",
        }
    }

    pub fn from_name(name: &str) -> Option<Pass> {
        Pass::ALL.into_iter().find(|pass| pass.name() == name)
    }

    /// The lowest level whose output the pass belongs to: a reconstruction of a source construct is `Source`.
    pub fn level(self) -> Level {
        match self >= Pass::Fields {
            true => Level::Source,
            false => Level::Structured,
        }
    }

    fn bit(self) -> u64 {
        1 << self as u32
    }
}

/// A set of passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PassSet(u64);

impl PassSet {
    pub fn of(passes: &[Pass]) -> PassSet {
        PassSet(passes.iter().fold(0, |bits, pass| bits | pass.bit()))
    }

    pub fn has(self, pass: Pass) -> bool {
        self.0 & pass.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The passes of the set, in the order of [`Pass::ALL`].
    pub fn iter(self) -> impl Iterator<Item = Pass> {
        Pass::ALL.into_iter().filter(move |pass| self.has(*pass))
    }
}

/// The level of the output and the passes that run for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Options {
    pub level: Level,
    pub passes: PassSet,
}

impl Options {
    /// Every pass that belongs to `level` or to a level below it; none at `Lifted`.
    pub fn at(level: Level) -> Options {
        let passes: Vec<Pass> = Pass::ALL
            .into_iter()
            .filter(|pass| level != Level::Lifted && pass.level() <= level)
            .collect();
        Options {
            level,
            passes: PassSet::of(&passes),
        }
    }

    /// The four first passes at `Structured`: what `decompile` has always run.
    pub fn baseline() -> Options {
        Options {
            level: Level::Structured,
            passes: PassSet::of(&Pass::BASELINE),
        }
    }

    pub fn on(&self, pass: Pass) -> bool {
        self.passes.has(pass)
    }

    /// Whether any of `passes` is on.
    pub fn any(&self, passes: &[Pass]) -> bool {
        passes.iter().any(|pass| self.on(*pass))
    }

    pub fn with(mut self, pass: Pass) -> Options {
        self.passes.0 |= pass.bit();
        self
    }

    pub fn without(mut self, pass: Pass) -> Options {
        self.passes.0 &= !pass.bit();
        self
    }
}

/// The most that can be proved: every pass, at `Source`.
impl Default for Options {
    fn default() -> Options {
        Options::at(Level::Source)
    }
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
