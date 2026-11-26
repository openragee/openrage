//! The inputs a function is checked on: a few seeds, more while one enters a block none had,
//! and then inputs chosen to turn a test towards a block no seed entered.
//!
//! A chosen input is a seed with one more of its made-up cells given: the cell that makes a
//! comparison come out the other way, the value of a case, a zero or a one. A run says which
//! cell that is where the value under a test is one draw, read as it is or with cells added.
//! ⚠ No cell is found for a value worked out of more: a mask, a product, a comparison kept in a
//! variable and tested later, one cell of a value of several, a constant the code itself sets.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use super::{operated, plain, run_on, Limits, Run, SEEDS, STALE};
use crate::gta6::script::cfg::{self, BlockId, Terminator};
use crate::gta6::script::ir::{BinOp, Expr, Kind};
use crate::gta6::script::Project;

/// A draw, by the number its cell is made up from, with what is added to that cell to give the value in hand.
pub(super) type Origin = (u64, u64);

/// The cells kept for one test, the times one block's test is noted in a run, and the turns one run keeps.
pub(super) const NOTED: usize = 8;
pub(super) const NOTES: u8 = 2;
pub(super) const TURNS: usize = 64;

/// The tries towards one block that do not enter it before no more are made for it.
const GIVEN_UP: u8 = 3;
/// The tries in a row that enter no block before no more are made.
const IDLE: u64 = 24;

/// What chose an input: a seed alone, or the kind of test a cell was chosen to turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Turned {
    #[default]
    Seed,
    /// `x == 0`, `x != 0` or `x` itself: a null test, a flag.
    Zero,
    /// `x == c` or `x != c`, for a constant that is not zero.
    Constant,
    /// `x < y` and the three like it: a range, a loop's bound.
    Bound,
    /// `x == y` or `x != y`, for two values.
    Values,
    /// The value of a case of a switch.
    Case,
}

impl Turned {
    pub(crate) const ALL: [Turned; 6] = [
        Turned::Seed,
        Turned::Zero,
        Turned::Constant,
        Turned::Bound,
        Turned::Values,
        Turned::Case,
    ];

    /// What a count of the blocks entered first by an input of this kind is called.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Turned::Seed => "blocks entered by a seed",
            Turned::Zero => "blocks entered by turning a test against zero",
            Turned::Constant => "blocks entered by turning an equality with a constant",
            Turned::Bound => "blocks entered by turning an ordering",
            Turned::Values => "blocks entered by turning an equality of two values",
            Turned::Case => "blocks entered by choosing a case",
        }
    }
}

/// What a count of the blocks no input entered is called, by what stands before the block.
// ⚠ A block past a test is counted there also where the run was cut before the test was made.
pub(crate) const LEFT: [&str; 5] = [
    "blocks not entered past a test no input turned",
    "blocks not entered at a case no input chose",
    "blocks not entered after a run was cut",
    "blocks not entered behind another not entered",
    "blocks not entered that nothing leads to",
];

/// One input: a seed, with the draws that are given a chosen cell in place of the one the seed makes up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Input {
    pub seed: u64,
    pub chosen: HashMap<u64, u64>,
    /// What the last cell was chosen to turn; a seed alone where none was.
    pub by: Turned,
}

impl Input {
    pub(crate) fn of(seed: u64) -> Input {
        Input {
            seed,
            ..Input::default()
        }
    }
}

impl fmt::Display for Input {
    // ⚠ No `, ` and no `: ` here: what a suite reads off a difference is cut at those.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "seed {}", self.seed)?;
        match self.chosen.len() {
            0 => Ok(()),
            cells => write!(f, " with {cells} chosen cell(s)"),
        }
    }
}

/// A cell that, given to a draw, would have sent a test of a run to the block `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    pub to: BlockId,
    pub draw: u64,
    pub cell: u64,
    pub by: Turned,
}

/// One side of a comparison as a run met it: its cell, the draw it is where it is one, and whether the code states it as a constant.
#[derive(Clone, Copy, Debug)]
pub(super) struct Side {
    pub cell: u64,
    pub from: Option<Origin>,
    pub constant: bool,
}

/// Whether a sum at the kind `at` wraps over the low bits of its cells, so that a draw under it is still that draw with a cell added.
pub(super) fn sums(at: &Kind) -> bool {
    let narrow = matches!(at, Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16);
    let wide = matches!(
        at,
        Kind::I32 | Kind::U32 | Kind::Hash | Kind::I64 | Kind::U64
    );
    narrow || wide
}

/// Whether the code states `e` as a constant, under any reading of it.
pub(super) fn constant(e: &Expr) -> bool {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } | Expr::Sugar { e: inner, .. } = e {
        e = &**inner;
    }
    matches!(e, Expr::Const(_))
}

/// The cells a comparison at `at` against `cell` may come out otherwise on: the cell itself and a step either side of it.
fn near(cell: u64, at: &Kind) -> [u64; 3] {
    match at {
        Kind::F32 => {
            let value = f32::from_bits(cell as u32);
            let (above, below) = ((value + 1.0).to_bits(), (value - 1.0).to_bits());
            [u64::from(cell as u32), u64::from(above), u64::from(below)]
        }
        Kind::F64 => {
            let value = f64::from_bits(cell);
            [cell, (value + 1.0).to_bits(), (value - 1.0).to_bits()]
        }
        _ => [cell, cell.wrapping_add(1), cell.wrapping_sub(1)],
    }
}

/// For the comparison `left op right` at `at`, each draw a side is with the cell to give it that makes the comparison come out the other way.
pub(super) fn turning(op: BinOp, at: &Kind, left: Side, right: Side) -> Vec<(u64, u64, Turned)> {
    let Some(now) = operated(op, at, left.cell, right.cell) else {
        return Vec::new();
    };
    let other = |a: u64, b: u64| {
        let then = operated(op, at, a, b);
        then.is_some_and(|then| (then != 0) != (now != 0))
    };
    let kind = |against: Side| match (op, against.constant, against.cell) {
        (BinOp::Eq | BinOp::Ne, true, 0) => Turned::Zero,
        (BinOp::Eq | BinOp::Ne, true, _) => Turned::Constant,
        (BinOp::Eq | BinOp::Ne, false, _) => Turned::Values,
        _ => Turned::Bound,
    };
    let mut out = Vec::new();
    if let Some((draw, add)) = left.from {
        let mut cells = near(right.cell, at).into_iter();
        let found = cells.find(|cell| other(*cell, right.cell));
        out.extend(found.map(|cell| (draw, plain(cell.wrapping_sub(add)), kind(right))));
    }
    if let Some((draw, add)) = right.from {
        let mut cells = near(left.cell, at).into_iter();
        let found = cells.find(|cell| other(left.cell, *cell));
        out.extend(found.map(|cell| (draw, plain(cell.wrapping_sub(add)), kind(left))));
    }
    out
}

/// What holds the runs of every input tried, where the inputs are chosen for a verdict on the forms and not for their runs.
pub(super) type Held<'h> = &'h mut dyn FnMut(&Input, &[Run]) -> Result<(), String>;

/// The inputs chosen for some forms of one function, and what they entered.
pub(super) struct Chosen {
    /// Each input kept, with the run of every form on it where the runs were asked for.
    pub kept: Vec<(Input, Vec<Run>)>,
    /// The blocks of each form that an input entered.
    pub reached: Vec<Vec<bool>>,
    /// The blocks entered first by an input of each kind, at the place of the kind in [`Turned::ALL`].
    pub entered: [u32; Turned::ALL.len()],
}

struct Search<'a> {
    forms: &'a [&'a cfg::Function],
    limits: Limits,
    project: &'a Project,
    singles: &'a BTreeSet<u32>,
    chosen: Chosen,
    /// The tries towards each block of each form that did not enter it.
    failed: Vec<Vec<u8>>,
    /// The turns still to try: the input kept that noted each, the form it ran, and the turn.
    open: VecDeque<(usize, usize, Turn)>,
}

impl Search<'_> {
    fn whole(&self) -> bool {
        let mut blocks = self.chosen.reached.iter().flatten();
        blocks.all(|entered| *entered)
    }

    /// Whether a turn towards `block` of form `form` is still worth a try.
    fn wanted(&self, form: usize, block: BlockId) -> bool {
        let entered = self.chosen.reached.get(form).and_then(|of| of.get(block));
        let failed = self.failed.get(form).and_then(|of| of.get(block));
        entered == Some(&false) && failed.is_some_and(|failed| *failed < GIVEN_UP)
    }

    /// Every form run on `input`.
    fn runs(&self, input: &Input) -> Vec<Run> {
        let forms = self
            .forms
            .iter()
            .zip(&self.chosen.reached)
            .zip(&self.failed);
        let runs = forms.map(|((form, reached), failed)| {
            // A run notes its tests only where turns are tried, and only towards a block worth a try.
            let wanted: Vec<bool> = match self.limits.turns {
                0 => Vec::new(),
                _ => {
                    let blocks = reached.iter().zip(failed);
                    blocks
                        .map(|(entered, failed)| !entered && *failed < GIVEN_UP)
                        .collect()
                }
            };
            run_on(
                form,
                input,
                &wanted,
                self.limits,
                self.project,
                self.singles,
            )
        });
        runs.collect()
    }

    /// Marks the blocks `runs` entered and counts those none had under what chose `input`; whether there is one.
    fn enters(&mut self, input: &Input, runs: &[Run]) -> bool {
        let mut news = 0;
        for (held, run) in self.chosen.reached.iter_mut().zip(runs) {
            for (before, now) in held.iter_mut().zip(&run.reached) {
                news += u32::from(*now && !*before);
                *before |= *now;
            }
        }
        if let Some(count) = self.chosen.entered.get_mut(input.by as usize) {
            *count += news;
        }
        news > 0
    }

    /// Keeps `input`, with its runs where `with_runs` asks, and takes the turns they noted.
    fn keep(&mut self, input: Input, mut runs: Vec<Run>, with_runs: bool) {
        let at = self.chosen.kept.len();
        for (form, run) in runs.iter_mut().enumerate() {
            let turns = std::mem::take(&mut run.turns);
            self.open
                .extend(turns.into_iter().map(|turn| (at, form, turn)));
        }
        if !with_runs {
            runs.clear();
        }
        self.chosen.kept.push((input, runs));
    }
}

/// Chooses the inputs for `forms` of one function. With no `verdict` the run of each input kept is kept with it, to hold other forms against; with one, every input tried is held as it says, and `None` is returned where a form does not run a seed.
// ⚠ The seeds come first and are tried exactly as they were before any input was chosen: what
// a suite found on them it still finds, and a chosen input can only add to it.
pub(super) fn choose(
    forms: &[&cfg::Function],
    limits: Limits,
    project: &Project,
    singles: &BTreeSet<u32>,
    mut verdict: Option<Held<'_>>,
) -> Result<Option<Chosen>, String> {
    let sized = |form: &&cfg::Function| form.cfg.blocks.len();
    let mut search = Search {
        forms,
        limits,
        project,
        singles,
        chosen: Chosen {
            kept: Vec::new(),
            reached: forms.iter().map(|form| vec![false; sized(form)]).collect(),
            entered: [0; Turned::ALL.len()],
        },
        failed: forms.iter().map(|form| vec![0; sized(form)]).collect(),
        open: VecDeque::new(),
    };
    let with_runs = verdict.is_none();
    let unrun = |runs: &[Run]| runs.iter().any(|run| run.unsupported.is_some());
    // The first seeds whatever they enter, then more while one enters a block none had.
    let mut stale = 0;
    for seed in 0..SEEDS + limits.more {
        if stale >= STALE || (seed >= SEEDS && search.whole()) {
            break;
        }
        let input = Input::of(seed);
        let runs = search.runs(&input);
        if let Some(held) = verdict.as_deref_mut() {
            if unrun(&runs) {
                return Ok(None);
            }
            held(&input, &runs)?;
        }
        if search.enters(&input, &runs) || seed < SEEDS {
            search.keep(input, runs, with_runs);
            stale = 0;
        } else {
            stale += 1;
        }
    }
    // Then the tests: each cell a kept run noted, given to the input that noted it.
    let (mut tries, mut idle) = (0, 0);
    while let Some((from, form, turn)) = search.open.pop_front() {
        if tries >= limits.turns || idle >= IDLE || search.whole() {
            break;
        }
        if !search.wanted(form, turn.to) {
            continue;
        }
        let Some((noted, _)) = search.chosen.kept.get(from) else {
            continue;
        };
        if noted.chosen.get(&turn.draw) == Some(&turn.cell) {
            continue;
        }
        let mut input = noted.clone();
        input.chosen.insert(turn.draw, turn.cell);
        input.by = turn.by;
        tries += 1;
        let runs = search.runs(&input);
        // What the oracle does not run of a chosen input says nothing: the input is dropped.
        let news = match unrun(&runs) {
            true => false,
            false => {
                if let Some(held) = verdict.as_deref_mut() {
                    held(&input, &runs)?;
                }
                search.enters(&input, &runs)
            }
        };
        if search.wanted(form, turn.to) {
            let failed = search.failed.get_mut(form);
            if let Some(failed) = failed.and_then(|of| of.get_mut(turn.to)) {
                *failed += 1;
            }
        }
        match news {
            true => {
                search.keep(input, runs, with_runs);
                idle = 0;
            }
            false => idle += 1,
        }
    }
    Ok(Some(search.chosen))
}

/// The blocks a terminator names, a block named twice given twice.
fn targets(term: &Terminator, out: &mut Vec<BlockId>) {
    match term {
        Terminator::Jump(to) => out.push(*to),
        Terminator::Branch { then, els, .. } => out.extend([*then, *els]),
        Terminator::Switch { cases, default, .. } => {
            out.extend(cases.iter().map(|case| case.1));
            out.push(*default);
        }
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

/// The blocks of `f` that `reached` says no input entered, counted by what stands before each, at its place in [`LEFT`].
pub(super) fn left(f: &cfg::Function, reached: &[bool]) -> [u32; LEFT.len()] {
    let blocks = &f.cfg.blocks;
    // The first place in `LEFT` that an entered block before each block gives it.
    let mut before = vec![usize::MAX; blocks.len()];
    let mut next = Vec::new();
    for (block, _) in blocks.iter().zip(reached).filter(|(_, entered)| **entered) {
        let place = match &block.term {
            Terminator::Branch { .. } => 0,
            Terminator::Switch { .. } => 1,
            Terminator::Jump(_) => 2,
            Terminator::Return(_) | Terminator::Unreachable => continue,
        };
        next.clear();
        targets(&block.term, &mut next);
        for to in &next {
            if let Some(first) = before.get_mut(*to) {
                *first = (*first).min(place);
            }
        }
    }
    // What the entry leads to, by any way.
    let mut led = vec![false; blocks.len()];
    let mut open = Vec::new();
    if !blocks.is_empty() {
        open.push(cfg::ENTRY);
    }
    while let Some(b) = open.pop() {
        let Some(seen) = led.get_mut(b) else {
            continue;
        };
        if !std::mem::replace(seen, true) {
            if let Some(block) = blocks.get(b) {
                targets(&block.term, &mut open);
            }
        }
    }
    let mut left = [0; LEFT.len()];
    let unentered = reached.iter().enumerate().filter(|(_, entered)| !**entered);
    for (b, _) in unentered {
        let place = match (before.get(b), led.get(b)) {
            (Some(place), _) if *place < 3 => *place,
            (_, Some(true)) => 3,
            _ => 4,
        };
        left[place] += 1;
    }
    left
}

mod tests {
    use super::super::{agree, check_blocks, function, Agreed};
    use super::*;
    use crate::gta6::script::{Context, Names, Options};

    /// The limits of a test written by hand, with turns tried.
    const TURNING: Limits = Limits {
        turns: 16,
        ..Limits::DEFAULT
    };

    /// A test of each parameter against a constant no seed makes up, the second inside the first.
    const NESTED: &str = "if (((as_i32(p0) == as_i32(77)))) { if (((as_i32(p1) == as_i32(1234)))) { Native.DEMO.USE(1); } else { Native.DEMO.USE(2); } } else { Native.DEMO.USE(3); } return 0;";

    fn side(cell: u64, from: Option<Origin>, constant: bool) -> Side {
        Side {
            cell,
            from,
            constant,
        }
    }

    #[test]
    fn a_comparison_is_turned_by_a_cell_for_the_side_that_is_a_draw() {
        let int = Kind::I32;
        // Unequal to a constant: the constant makes it equal.
        let turned = turning(
            BinOp::Eq,
            &int,
            side(3, Some((7, 0)), false),
            side(5, None, true),
        );
        assert_eq!(turned, [(7, 5, Turned::Constant)]);
        // The draw with three added is the value: the cell chosen is three under the constant.
        let turned = turning(
            BinOp::Eq,
            &int,
            side(3, Some((7, 3)), false),
            side(5, None, true),
        );
        assert_eq!(turned, [(7, 2, Turned::Constant)]);
        // Set, and tested against zero: zero clears it.
        let turned = turning(
            BinOp::Ne,
            &int,
            side(6, Some((7, 0)), false),
            side(0, None, true),
        );
        assert_eq!(turned, [(7, 0, Turned::Zero)]);
        // Not under a bound that is a value: one under it is.
        let turned = turning(
            BinOp::Lt,
            &int,
            side(9, Some((7, 0)), false),
            side(4, None, false),
        );
        assert_eq!(turned, [(7, 3, Turned::Bound)]);
        // The bound is the draw: one past the other side.
        let turned = turning(
            BinOp::Lt,
            &int,
            side(9, None, false),
            side(4, Some((8, 0)), false),
        );
        assert_eq!(turned, [(8, 10, Turned::Bound)]);
        // Two draws: a cell for each.
        let both = turning(
            BinOp::Ne,
            &int,
            side(1, Some((7, 0)), false),
            side(1, Some((8, 0)), false),
        );
        assert_eq!(both, [(7, 2, Turned::Values), (8, 2, Turned::Values)]);
        // A float is stepped as a float.
        let (one, two, three) = (1.0f32.to_bits(), 2.5f32.to_bits(), 3.5f32.to_bits());
        let float = turning(
            BinOp::Gt,
            &Kind::F32,
            side(one.into(), Some((7, 0)), false),
            side(two.into(), None, true),
        );
        assert_eq!(float, [(7, u64::from(three), Turned::Bound)]);
        // No draw, or an operator the oracle does not work at the kind: nothing to choose.
        assert!(turning(BinOp::Eq, &int, side(3, None, false), side(5, None, true)).is_empty());
        let unknown = turning(
            BinOp::Lt,
            &Kind::Unknown,
            side(3, Some((7, 0)), false),
            side(5, None, true),
        );
        assert!(unknown.is_empty());
    }

    #[test]
    fn a_run_notes_the_cell_that_turns_its_test_and_the_cell_turns_it() {
        let f = function(
            "if (((as_i32(p0) == as_i32(77)))) { Native.DEMO.USE(1); } else { Native.DEMO.USE(2); } return 0;",
        );
        let wanted = vec![true; f.cfg.blocks.len()];
        let none = Project::empty();
        let seed = Input::of(0);
        let run = run_on(&f, &seed, &wanted, Limits::DEFAULT, none, &BTreeSet::new());
        let [turn] = run.turns.as_slice() else {
            panic!("one turn is noted: {:?}", run.turns);
        };
        assert_eq!((turn.cell, turn.by), (77, Turned::Constant));
        assert!(!run.reached[turn.to]);
        let mut chosen = seed.clone();
        chosen.chosen.insert(turn.draw, turn.cell);
        let turned = run_on(&f, &chosen, &[], Limits::DEFAULT, none, &BTreeSet::new());
        assert!(turned.reached[turn.to]);
        // Asked for no block, a run notes nothing.
        assert!(
            run_on(&f, &seed, &[], Limits::DEFAULT, none, &BTreeSet::new())
                .turns
                .is_empty()
        );
        assert!(turned.turns.is_empty());
    }

    #[test]
    fn inputs_are_chosen_for_the_blocks_no_seed_enters() {
        let f = function(NESTED);
        let names = Names::default();
        let cx = Context::baseline(&names);
        let seeds = check_blocks(&f, &cx, Limits::DEFAULT).expect("the passes keep it");
        assert!(seeds.reached < seeds.blocks, "{seeds:?}");
        assert_eq!(u64::from(seeds.seeds), SEEDS);
        // One arm is past a test no seed turned, and the two under it behind that arm.
        assert_eq!((seeds.left[0], seeds.left[3]), (1, 2), "{seeds:?}");
        let turned = check_blocks(&f, &cx, TURNING).expect("the passes keep it");
        assert_eq!(turned.reached, turned.blocks, "{turned:?}");
        assert_eq!(turned.left, [0; LEFT.len()]);
        assert_eq!(turned.entered[Turned::Constant as usize], 3, "{turned:?}");
        assert_eq!(u64::from(turned.seeds), SEEDS + 2);
        // Every pass is held to what the function does on the chosen inputs too.
        let every = Context::new(&names, Project::empty(), Options::default());
        let all = check_blocks(&f, &every, TURNING).expect("every pass keeps it");
        assert_eq!(all.reached, all.blocks);
    }

    #[test]
    fn a_case_is_chosen_through_the_difference_the_switch_is_on() {
        let f = function(
            "switch (as_i32(((as_i32(p0) - 3)))) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; case 40: return 5; default: break; } return p1;",
        );
        let names = Names::default();
        let cx = Context::baseline(&names);
        let turned = check_blocks(&f, &cx, TURNING).expect("the passes keep it");
        assert_eq!(turned.reached, turned.blocks, "{turned:?}");
        assert!(turned.entered[Turned::Case as usize] >= 1, "{turned:?}");
    }

    #[test]
    fn a_difference_past_a_test_no_seed_turns_is_found_by_a_chosen_input() {
        let was = function(NESTED);
        let is = function(&NESTED.replace("USE(1)", "USE(9)"));
        let none = Project::empty();
        // The seeds enter neither form's first arm: the two agree as far as they went.
        let unseen = Agreed {
            skipped: false,
            whole: false,
        };
        assert_eq!(agree(&was, &is, Limits::DEFAULT, none), Ok(unseen));
        let found = agree(&was, &is, TURNING, none).expect_err("the arm differs");
        assert!(found.starts_with("seed "), "{found}");
        assert!(found.contains(" chosen cell(s): event "), "{found}");
        // What reads a difference cuts it at the first `, ` and then at `: `.
        let how = found
            .split_once(", ")
            .and_then(|(_, rest)| rest.split_once(": "));
        assert_eq!(how.map(|(how, _)| how), Some("an argument"));
        let whole = Agreed {
            skipped: false,
            whole: true,
        };
        assert_eq!(agree(&was, &was.clone(), TURNING, none), Ok(whole));
    }

    #[test]
    fn a_draw_is_followed_through_a_variable_a_reading_and_a_sum() {
        // The value tested is a call's cell, kept in a temporary, read as a byte and stepped.
        let f = function(
            "v_000010_0 = Native.DEMO.GET(p0); local_1 = v_000010_0; if (((as_i32(((as_i32(as_u8(local_1)) + as_i32(2)))) == as_i32(9)))) { Native.DEMO.USE(1); } else { } return 0;",
        );
        let names = Names::default();
        let cx = Context::baseline(&names);
        let turned = check_blocks(&f, &cx, TURNING).expect("the passes keep it");
        assert_eq!(turned.reached, turned.blocks, "{turned:?}");
        // A variable the code itself sets is no draw: nothing turns a test of it.
        let set = function(
            "local_1 = 5; if (((as_i32(local_1) == as_i32(9)))) { Native.DEMO.USE(1); } else { } return 0;",
        );
        let stays = check_blocks(&set, &cx, TURNING).expect("the passes keep it");
        assert!(stays.reached < stays.blocks, "{stays:?}");
        assert_eq!(u64::from(stays.seeds), SEEDS);
    }

    #[test]
    fn an_input_is_written_with_no_mark_a_difference_is_cut_at() {
        let mut input = Input::of(3);
        assert_eq!(input.to_string(), "seed 3");
        input.chosen.insert(1, 2);
        assert_eq!(input.to_string(), "seed 3 with 1 chosen cell(s)");
    }
}
