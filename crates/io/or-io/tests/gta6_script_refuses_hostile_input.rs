//! A lifted listing or a compiled script a user picks must never take the process down with it.
//!
//! Each case is an `#[ignore]`d test re-run by name in a child process, as in
//! `a_hostile_archive_is_refused.rs`: a stack overflow and an allocation the allocator refuses
//! are **aborts**, which `catch_unwind` does not see and which would take this binary with them.
//! The child counts what it holds, so a count the input states and the reader believed shows as
//! memory far past the input's size.
//!
//! ⚠ The parent asserts on the child's **exit status**, never on `is_err()` alone: a refusal and
//! a crash are both "not read".
//!
//! Every input is written here by hand or built in code. Slow tier: the deep and the long cases
//! are megabytes of text.
//!
//! A compiled program is also given code that is hostile in a container that reads: what holds
//! there is a count of steps, of cases or of operands copied, never the size of the input.

#![cfg(all(feature = "gta5", feature = "gta6"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use or_io::gta6::script::program::Program;
use or_io::gta6::script::{decompile, decompile_program, Context, Names, Options, Project};

/// The bytes held now, and the most held since a case began.
static HELD: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every request goes to the system's allocator as it came; the counts are only kept.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let held = HELD.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(held, Ordering::Relaxed);
        // SAFETY: the caller's contract for `alloc` is the system allocator's own.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from `alloc` above with this layout, which is the system's.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What a case may hold beyond its input's size many times over.
// ⚠ A ceiling against a count believed into memory, which is gigabytes; no measurement of what
// reading costs is stated here.
const TIMES: usize = 64;
const FLOOR: usize = 64 << 20;

/// The stack the pipeline is given: what a reader that bounds its nesting needs, and no more.
const STACK: usize = 64 << 20;

/// Runs `case`: what it made, and the most bytes it held beyond those held when it began.
fn held_by<T>(case: impl FnOnce() -> T) -> (T, usize) {
    let before = HELD.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let made = case();
    let held = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    (made, held)
}

/// Runs `case` and asserts that it held no more than an input of `input` bytes allows.
fn bounded<T>(input: usize, case: impl FnOnce() -> T) -> T {
    let (made, held) = held_by(case);
    assert!(
        held <= TIMES * input + FLOOR,
        "{held} bytes held for an input of {input} bytes"
    );
    made
}

/// Whether `text` decompiles, on a stack of [`STACK`] bytes and within what its size allows.
fn reads(text: String) -> bool {
    let size = text.len();
    let reader = std::thread::Builder::new().stack_size(STACK);
    let run = move || decompile(&text, &Names::default()).is_ok();
    let read = bounded(size, || reader.spawn(run).expect("a thread starts").join());
    read.expect("a refusal is an error, never a panic")
}

/// A listing of one function, written by hand; a case puts in `LOCALS`, `RETURNS` and `BODY`.
const LISTING: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static VmValue func_000010(VmValue arg0, VmValue arg1) // 0x000010
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[LOCALS];
        VmValue v_000010_0 = default; VmValues v_000028_2 = null!; VmValues v_000030_3 = null!;
        VmValue stack_000040_0 = default;
BODY        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static VmValues PAIR() => Vm.Native(0x0000000000000002UL, 0x00000002U, RETURNS);
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
        }
    }
}
"#;

fn listing(locals: &str, returns: &str, body: &str) -> String {
    let text = LISTING.replace("LOCALS", locals);
    text.replace("RETURNS", returns).replace("BODY", body)
}

fn with_body(body: &str) -> String {
    listing("4", "2", body)
}

/// The lines of a body, each at the depth a function's statements stand at.
fn lines(of: &[&str]) -> String {
    of.iter().map(|line| format!("        {line}\n")).collect()
}

/// A body that reads: a call, its result used, and a return.
fn plain() -> String {
    lines(&[
        "v_000010_0 = Native.DEMO.GET(p0);",
        "Native.DEMO.USE(v_000010_0);",
        "return p1;",
    ])
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_a_listing_cut_short() {
    let whole = with_body(&plain());
    assert!(reads(whole.clone()), "the listing itself is refused");
    // Every line end, and 256 places inside the lines.
    let end = whole.trim_end().len();
    let ends = whole.match_indices('\n').map(|(at, _)| at + 1);
    let mut cuts: Vec<usize> = ends.filter(|at| *at < end).collect();
    cuts.extend((1..=256).map(|n| n * (end - 1) / 257).filter(|at| *at > 0));
    let read: Vec<usize> = cuts
        .iter()
        .copied()
        .filter(|at| reads(whole[..*at].to_owned()))
        .collect();
    eprintln!(
        "child: a listing of {} bytes cut at {} places, {} of them read as a listing",
        whole.len(),
        cuts.len(),
        read.len()
    );
    assert!(read.is_empty(), "cut at {read:?} the listing still reads");
}

/// A body that nests `levels` deep, in the way `kind` names.
fn nested(kind: &str, levels: usize) -> String {
    let open = |text: &str| text.repeat(levels);
    let assigned = |value: String| lines(&[&format!("local_0 = {value};"), "return local_0;"]);
    match kind {
        "parentheses" => assigned(format!("{}1{}", open("("), open(")"))),
        "casts" => assigned(format!("{}p0{}", open("as_i32("), open(")"))),
        "loads" => assigned(format!("{}p0{}", open("load_u64("), open(", 0x8)"))),
        "negations" => assigned(format!("{}p0", open("!"))),
        "ifs" => {
            let head = lines(&["if (((as_u32(p0) == 0)))", "{"]);
            let tail = lines(&["}", "else", "{", "}"]);
            let inner = lines(&["local_0 = 1;"]);
            format!(
                "{}{inner}{}{}",
                open(&head),
                open(&tail),
                lines(&["return local_0;"])
            )
        }
        _ => {
            let head = lines(&["while (Truth(true))", "{"]);
            let inner = lines(&["break;"]);
            let tail = lines(&["}"]);
            format!(
                "{}{inner}{}{}",
                open(&head),
                open(&tail),
                lines(&["return local_0;"])
            )
        }
    }
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_listings_nested_a_hundred_thousand_deep() {
    for kind in [
        "parentheses",
        "casts",
        "loads",
        "negations",
        "ifs",
        "whiles",
    ] {
        // A few deep reads, so that what is refused below is the depth and not the form.
        assert!(
            reads(with_body(&nested(kind, 8))),
            "{kind}: 8 deep is refused"
        );
        let text = with_body(&nested(kind, 100_000));
        eprintln!("child: {kind}, 100,000 deep in {} bytes", text.len());
        assert!(!reads(text), "{kind}: 100,000 deep was followed to its end");
    }
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_counts_of_four_billion() {
    // A few bytes of text, and gigabytes where a reader or a pass sizes anything by them.
    for big in ["4294967295", "4000000000", "0x7FFFFFFF", "4294967296"] {
        let zeroed = format!("v_000030_3 = zero_slots({big});");
        let cleared = format!("zero_bytes(Vm.Address(locals, 0), {big});");
        let loaded = format!("v_000028_2 = load_bytes(p0, 0x8, {big});");
        let stored = format!("store_bytes(p0, 0x8, {big}, new VmValues(p0, p1));");
        let extracted = format!("v_000028_2 = extract_bytes(new VmValues(p0, p1), {big}, {big});");
        let addressed = format!("Native.DEMO.USE(Vm.Address(locals, {big}));");
        let cell = format!("return v_000028_2[{big}];");
        let offset = format!("return load_u64(p0, {big});");
        let cases = [
            listing(big, "2", &lines(&["return local_0;"])),
            listing("4", big, &plain()),
            with_body(&lines(&[&zeroed, "return v_000030_3[0];"])),
            with_body(&lines(&[&cleared, "return local_0;"])),
            with_body(&lines(&[&loaded, "return v_000028_2[0];"])),
            with_body(&lines(&[&stored, "return p1;"])),
            with_body(&lines(&[&extracted, "return v_000028_2[0];"])),
            with_body(&lines(&[&addressed, "return p1;"])),
            with_body(&lines(&[&cell])),
            with_body(&lines(&[&offset])),
        ];
        let read = cases.into_iter().filter(|text| reads(text.clone())).count();
        eprintln!("child: a count of {big} in 10 places, {read} read and the rest refused");
    }
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_indices_nothing_declares() {
    let twice = lines(&[
        "switch (as_i32(p0))",
        "{",
        "case 1:",
        "goto label_000070;",
        "case 1:",
        "goto label_000070;",
        "default:",
        "goto label_000070;",
        "}",
        "label_000070: ;",
        "return p1;",
    ]);
    // The function a second time, at the offset it already has.
    let whole = with_body(&plain());
    let function = whole.find("    [VmFunction").expect("the function starts");
    let classes = whole
        .find("    public static class Native")
        .expect("the classes start");
    let mut doubled = whole[..classes].to_owned();
    doubled.push_str(&whole[function..]);
    let cases = [
        (
            "a parameter past the two there are",
            with_body(&lines(&["return p9;"])),
        ),
        (
            "a local past the four there are",
            with_body(&lines(&["return local_9;"])),
        ),
        (
            "a cell of a value of one cell",
            with_body(&lines(&["return v_000010_0[3];"])),
        ),
        (
            "a temporary nothing declares",
            with_body(&lines(&["v_0000AA_0 = 1;", "return v_0000AA_0;"])),
        ),
        ("a case twice in one switch", with_body(&twice)),
        ("a function offset twice in one listing", doubled),
    ];
    let read: Vec<&str> = cases
        .into_iter()
        .filter(|(_, text)| reads(text.clone()))
        .map(|(what, _)| what)
        .collect();
    assert!(read.is_empty(), "read as if it were stated: {read:?}");
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_ten_thousand_jumps() {
    const JUMPS: usize = 10_000;
    // A ring of labels, each jumping to the next and the last to the first.
    let ring: String = (0..JUMPS)
        .map(|n| {
            let (at, to) = (0x20_0000 + n, 0x20_0000 + (n + 1) % JUMPS);
            lines(&[
                &format!("label_{at:06X}: ;"),
                &format!("goto label_{to:06X};"),
            ])
        })
        .collect();
    // A row of tests that all jump to one label.
    let test = lines(&[
        "if (((as_u32(p0) == 0)))",
        "{",
        "goto label_000070;",
        "}",
        "else",
        "{",
        "}",
    ]);
    let row = format!(
        "{}{}",
        test.repeat(JUMPS),
        lines(&["return local_0;", "label_000070: ;", "return p1;"])
    );
    for (what, body) in [("a ring of labels", ring), ("tests to one label", row)] {
        let text = with_body(&body);
        eprintln!("child: {what}, {JUMPS} of them in {} bytes", text.len());
        // ⚠ No clock is read: a cost that grows as the square of this ends at the tier's timeout.
        assert!(reads(text), "{what} is refused");
    }
}

const BASE: u64 = 0x5000_0000;
const NONE: u32 = u32::MAX;
const HEADER: usize = 0x180;
/// The byte of the pooled data from which a case's own data stands.
const DATA: usize = 64;

/// The instructions the cases are written in, by their numbers.
const NOP: u16 = 0;
const PUSH_PARAM_0: u16 = 2;
const PUSH_1: u16 = 34;
const PUSH_I32: u16 = 46;
const DUP: u16 = 53;
/// One operand byte: the parameter cells above the local cells asked beyond the frame's own.
const ENTER: u16 = 131;
const RETURN: u16 = 144;
const RETURN_1: u16 = 145;
/// A jump's operand is a distance of four bytes, counted from the operand itself.
const JUMP: u16 = 156;
/// Two operands of four bytes: the cases, and where in the pooled data their distances stand.
const SWITCH: u16 = 158;
const BRZ: u16 = 159;
/// A distance of one byte.
const BRZ_SHORT: u16 = 160;
const BRNZ: u16 = 162;
const ADD_I32: u16 = 250;
const NEG_I32: u16 = 302;

/// One instruction as the code keeps it: its number as a word, then each operand byte under the key of its place.
fn insn(opcode: u16, operands: &[u8]) -> Vec<u8> {
    let mut bytes = opcode.wrapping_mul(0x843D).to_le_bytes().to_vec();
    let stored = operands.iter().enumerate().map(|(place, plain)| {
        let key = 0x85 ^ 0x11u8.wrapping_mul(place as u8);
        plain.wrapping_mul(0x65) ^ key
    });
    bytes.extend(stored);
    bytes
}

/// A function of one parameter that is `body` and a return.
fn of_one_parameter(body: &[u8]) -> Vec<u8> {
    let mut code = insn(ENTER, &[0x10]);
    code.extend_from_slice(body);
    code.extend(insn(RETURN, &[]));
    code
}

/// Loops nested `levels` deep: a start for each, then a test for each that jumps back to its start, the innermost first.
fn loops(levels: usize) -> Vec<u8> {
    let first = insn(ENTER, &[0x10]).len();
    let mut body = insn(NOP, &[]).repeat(levels);
    for level in (0..levels).rev() {
        body.extend(insn(PUSH_PARAM_0, &[]));
        let operand = first + body.len() + 2;
        let back = (first + 2 * level) as i64 - operand as i64;
        body.extend(insn(BRNZ, &(back as i32).to_le_bytes()));
    }
    of_one_parameter(&body)
}

/// Tests nested `levels` deep: each jumps ahead, past what it guards, to a landing of its own.
fn ladder(levels: usize) -> Vec<u8> {
    let first = insn(ENTER, &[0x10]).len();
    // Past the tests and the one instruction they all guard: a landing for each, the innermost first.
    let landings = first + 8 * levels + 2;
    let mut body = Vec::new();
    for level in 0..levels {
        body.extend(insn(PUSH_PARAM_0, &[]));
        let operand = first + body.len() + 2;
        let ahead = landings + 2 * (levels - 1 - level) - operand;
        body.extend(insn(BRZ, &(ahead as i32).to_le_bytes()));
    }
    body.extend(insn(NOP, &[]).repeat(levels + 1));
    of_one_parameter(&body)
}

/// What a program of hostile code may hold, whatever its size.
// ⚠ A ceiling against a count believed into memory, which is gigabytes for code of a few hundred
// bytes; no measurement of what a bound of the readers costs is stated here.
const CODE_CEILING: usize = 1 << 30;

/// What a program decompiled to: its functions, those of them written with no body, and the bytes written.
type Written = (usize, usize, usize);

/// Decompiles a program at the source level on a stack of [`STACK`] bytes: what it came to or the refusal, and the most bytes held.
fn decompiles(bytes: Vec<u8>) -> (Result<Written, String>, usize) {
    let reader = std::thread::Builder::new().stack_size(STACK);
    let run = move || -> Result<Written, String> {
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), Options::default());
        let done = decompile_program(&bytes, &cx).map_err(|why| why.to_string())?;
        let written: usize = done.files.iter().map(|file| file.text.len()).sum();
        Ok((done.stats.functions, done.stats.incomplete, written))
    };
    let (read, held) = held_by(|| reader.spawn(run).expect("a thread starts").join());
    (read.expect("a refusal is an error, never a panic"), held)
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_programs_whose_code_is_hostile() {
    let (push, test) = (insn(PUSH_PARAM_0, &[]), insn(BRZ_SHORT, &[1]));
    let switch = |cases: u32| {
        let operands = [cases.to_le_bytes(), (DATA as u32).to_le_bytes()].concat();
        [push.clone(), insn(SWITCH, &operands)].concat()
    };
    // A table of one case more than a decoder holds: every case goes to the return after its switch.
    let table = 8i32.to_le_bytes().repeat((1 << 16) + 1);
    let doubled = [insn(DUP, &[]), insn(ADD_I32, &[])].concat().repeat(40);
    let doubling = [insn(PUSH_1, &[]), doubled, insn(RETURN_1, &[])].concat();
    let negated = [push.clone(), insn(NEG_I32, &[]).repeat(100_000)].concat();
    let negations = [insn(ENTER, &[0x10]), negated, insn(RETURN_1, &[])].concat();
    let pushes = [insn(PUSH_1, &[]).repeat(1_000_000), insn(RETURN, &[])].concat();
    let tests = [push.clone(), test].concat().repeat(70_000);
    let inside = [
        insn(JUMP, &5i32.to_le_bytes()),
        insn(PUSH_I32, &7i32.to_le_bytes()),
        insn(RETURN_1, &[]),
    ];
    let cases: [(&str, Vec<u8>, &[u8]); 16] = [
        ("a cell doubled forty times", doubling, &[]),
        ("a hundred thousand negations of one cell", negations, &[]),
        ("a million cells pushed and left", pushes, &[]),
        (
            "seventy thousand tests in a row",
            of_one_parameter(&tests),
            &[],
        ),
        ("loops nested two thousand deep", loops(2000), &[]),
        ("loops nested forty thousand deep", loops(40_000), &[]),
        ("tests nested two thousand deep", ladder(2000), &[]),
        ("tests nested forty thousand deep", ladder(40_000), &[]),
        (
            "a switch of the most cases a decoder holds",
            of_one_parameter(&switch(1 << 16)),
            &table,
        ),
        (
            "a switch of one case more",
            of_one_parameter(&switch((1 << 16) + 1)),
            &table,
        ),
        (
            "a switch that states four billion cases",
            of_one_parameter(&switch(u32::MAX)),
            &table,
        ),
        (
            "twenty switches over one table",
            of_one_parameter(&switch(1 << 16).repeat(20)),
            &table,
        ),
        ("a jump to itself", insn(JUMP, &(-2i32).to_le_bytes()), &[]),
        (
            "a jump out of the code",
            [insn(JUMP, &1000i32.to_le_bytes()), insn(RETURN, &[])].concat(),
            &[],
        ),
        ("a jump into an operand", inside.concat(), &[]),
        (
            "a sum over an empty stack",
            [insn(ADD_I32, &[]), insn(RETURN_1, &[])].concat(),
            &[],
        ),
    ];
    for (what, code, data) in cases {
        let program = program_of(&code, data);
        // The container reads: what is hostile is the code, and only the code's readers meet it.
        let read = Program::read(&program);
        assert!(
            read.is_ok(),
            "{what}: the program itself is refused: {read:?}"
        );
        let (done, held) = decompiles(program);
        let came = match &done {
            Ok((functions, bare, written)) => {
                format!("{functions} function(s), {bare} with no body, {written} byte(s) written")
            }
            Err(why) => format!("refused: {why}"),
        };
        eprintln!(
            "child: {what}, {} byte(s) of code: {came}; {held} byte(s) held",
            code.len()
        );
        assert!(held <= CODE_CEILING, "{what}: {held} bytes held");
    }
    // Code that ends inside an instruction is no code: the walk for its functions refuses the program.
    let cut = program_of(&insn(PUSH_I32, &7i32.to_le_bytes())[..4], &[]);
    assert!(
        Program::read(&cut).is_err(),
        "code cut inside an instruction reads"
    );
    let (done, held) = decompiles(cut);
    assert!(
        done.is_err() && held <= CODE_CEILING,
        "{done:?}, {held} bytes held"
    );
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// Appends a section to a program being built and writes the pointer to it at `at`.
fn section(file: &mut Vec<u8>, at: usize, bytes: &[u8]) {
    let pointer = BASE + file.len() as u64;
    file.extend_from_slice(bytes);
    put64(file, at, pointer);
}

/// The least a compiled program is: one function that returns.
fn small_program() -> Vec<u8> {
    program_of(&insn(RETURN, &[]), &[])
}

/// A compiled program of the code `code`: a header, pooled data with `data` from its byte 64 on, the program's own row of the import table and one type.
fn program_of(code: &[u8], data: &[u8]) -> Vec<u8> {
    let mut file = vec![0; HEADER];
    put32(&mut file, 0x10, 0xABCD_0001);
    put32(
        &mut file,
        0x24,
        or_io::gta6::script::compiled::check_copy(0xABCD_0001),
    );
    put32(&mut file, 0x14, 1);
    put32(&mut file, 0x18, 0x6F);
    put64(&mut file, 0x1C, 0x5936_CAB2_EADB_CE67);
    for none in [0xBC, 0xC0, 0x130] {
        put32(&mut file, none, NONE);
    }
    // The pooled data: a zero, and at byte 4 the end of a base list that names nothing.
    let mut pooled = vec![0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    pooled.resize(DATA, 0);
    pooled.extend_from_slice(data);
    put32(&mut file, 0xFC, pooled.len() as u32);
    section(&mut file, 0xF0, &pooled);
    put32(&mut file, 0xF8, code.len() as u32);
    section(&mut file, 0xE8, code);
    let words: Vec<u8> = [0u32, 0, 1, 0]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    section(&mut file, 0x08, &words);
    // The import table holds the program itself first.
    put32(&mut file, 0x140, 1);
    put32(&mut file, 0x144, 1);
    section(&mut file, 0x148, &[0; 24]);
    // No native: two empty tables, each with its pointer.
    section(&mut file, 0xA8, &[]);
    section(&mut file, 0xB0, &[]);
    // One type with no base type, an empty base list and no vtable.
    let list = BASE + (HEADER + 4) as u64;
    let mut ty = vec![0; 136];
    put64(&mut ty, 0x10, 1);
    put32(&mut ty, 0x20, NONE);
    put32(&mut ty, 0x34, NONE);
    put64(&mut ty, 0x50, BASE);
    put64(&mut ty, 0x58, list);
    put64(&mut ty, 0x60, list);
    put32(&mut file, 0x28, 1);
    put32(&mut file, 0x2C, 1);
    section(&mut file, 0x30, &ty);
    file
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_a_program_cut_at_every_byte() {
    let whole = small_program();
    let read = bounded(whole.len(), || Program::read(&whole));
    assert!(read.is_ok(), "the program itself is refused: {read:?}");
    let read: Vec<usize> = (0..whole.len())
        .filter(|at| bounded(whole.len(), || Program::read(&whole[..*at])).is_ok())
        .collect();
    eprintln!(
        "child: a program of {} bytes cut at every byte, {} cuts read as a program",
        whole.len(),
        read.len()
    );
    assert!(read.is_empty(), "cut at {read:?} the program still reads");
}

#[test]
#[ignore = "run in a subprocess by the case that owns it"]
fn child_reads_a_program_with_every_word_set_wrong() {
    let whole = small_program();
    let len = whole.len();
    let (mut read, mut refused) = (0, 0);
    let mut tried = |bytes: &[u8]| match bounded(len, || Program::read(bytes)) {
        Ok(_) => read += 1,
        Err(_) => refused += 1,
    };
    for at in (0..len - 3).step_by(2) {
        // A count or an offset: the most a word holds, the file's length and either side of it.
        for word in [NONE, NONE - 1, 0x7FFF_FFFF, len as u32, len as u32 - 1, 1] {
            let mut bytes = whole.clone();
            put32(&mut bytes, at, word);
            tried(&bytes);
        }
    }
    for at in (0..len - 7).step_by(2) {
        // A pointer: to itself, which is a ring, to the header, to the end and past it, and to nowhere.
        let itself = BASE + at as u64;
        for word in [
            itself,
            BASE,
            BASE + len as u64,
            BASE + len as u64 + 1,
            BASE - 1,
            u64::MAX,
        ] {
            let mut bytes = whole.clone();
            put64(&mut bytes, at, word);
            tried(&bytes);
        }
    }
    eprintln!(
        "child: a program of {len} bytes with a word set wrong: {read} read, {refused} refused"
    );
}

fn assert_survived(test_name: &str, what: &str) {
    let exe = std::env::current_exe().expect("locate this test binary");
    let args = [
        test_name,
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ];
    let out = Command::new(exe).args(args).output().expect("spawn child");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stderr.lines().filter(|line| line.starts_with("child: ")) {
        println!("{line}");
    }
    assert!(
        out.status.success(),
        "{what}: {:?}\nA malformed input must end in an error, in memory its size bounds, and \
         never in a dead process.\n--- child stderr ---\n{stderr}",
        out.status,
    );
}

#[test]
fn a_listing_cut_anywhere_is_refused() {
    assert_survived("child_reads_a_listing_cut_short", "a listing cut short");
}

#[test]
fn a_listing_nested_a_hundred_thousand_deep_is_refused_not_followed() {
    assert_survived(
        "child_reads_listings_nested_a_hundred_thousand_deep",
        "a listing nested 100,000 deep",
    );
}

#[test]
fn a_count_of_four_billion_is_never_believed_into_memory() {
    assert_survived(
        "child_reads_counts_of_four_billion",
        "a listing that states a count of four billion",
    );
}

#[test]
fn an_index_nothing_declares_is_refused() {
    assert_survived(
        "child_reads_indices_nothing_declares",
        "a listing that names what it does not declare",
    );
}

#[test]
fn ten_thousand_jumps_are_read_to_their_end() {
    assert_survived(
        "child_reads_ten_thousand_jumps",
        "a listing of ten thousand jumps",
    );
}

#[test]
fn a_program_whose_code_is_hostile_is_refused_or_read_within_bounds() {
    assert_survived(
        "child_reads_programs_whose_code_is_hostile",
        "a compiled program whose code is hostile",
    );
}

#[test]
fn a_program_cut_at_any_byte_is_refused() {
    assert_survived(
        "child_reads_a_program_cut_at_every_byte",
        "a compiled program cut short",
    );
}

#[test]
fn a_program_with_a_count_or_a_pointer_set_wrong_is_read_or_refused() {
    assert_survived(
        "child_reads_a_program_with_every_word_set_wrong",
        "a compiled program with one word set wrong",
    );
}
