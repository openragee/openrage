use super::*;
use crate::gta6::hash::name_hash64;
use crate::gta6::script::ir::TypeKey;
use crate::gta6::script::program::{FieldRow, TypeRow};
use crate::hash::literal_string_hash;

const WORDS: [&str; 4] = ["grommet", "Quill", "kestrel", "UQ"];

fn folded32(name: &str) -> u32 {
    literal_string_hash(name.to_ascii_lowercase().as_bytes())
}

/// Seeks the type or field named `name`, as a program's record of it would have it sought.
fn member(targets: &mut Targets, name: &str) {
    targets.member(literal_hash64(name.as_bytes()), folded32(name));
}

/// What a search over [`WORDS`] is asked for: a name of each shape and case, and three it cannot find.
fn targets() -> Targets {
    let mut targets = Targets::default();
    for name in [
        "QuillGrommet",
        "_kestrelQuill",
        "GROMMET_KESTREL",
        "IQuill",
        "m_UQGrommet",
        // No join of the words.
        "Thimble",
    ] {
        member(&mut targets, name);
    }
    let program = "kestrelgrommet";
    targets.program(folded32(program), name_hash64(program.as_bytes()));
    // The 32-bit hash of a join beside a 64-bit hash that is not its own: a hit by chance.
    targets.member(
        literal_hash64(b"QuillKestrel") ^ 1,
        folded32("QuillKestrel"),
    );
    // A program's name is in lower case both times: a spelling with a capital is not it.
    targets.program(folded32("uqquill"), literal_hash64(b"UQQuill"));
    targets
}

fn names_of(search: &Search) -> Vec<&str> {
    search
        .found
        .iter()
        .map(|found| found.name.as_str())
        .collect()
}

#[test]
fn a_join_of_words_is_found_only_where_both_hashes_are_its_own() {
    let targets = targets();
    assert_eq!(targets.len(), 9);
    let budget = Budget {
        join: 2,
        candidates: 1 << 20,
    };
    let search = search(WORDS, &targets, budget);
    assert!(search.complete);
    // Every word alone in the four shapes one word has, and every pair in the five.
    let words = WORDS.len() as u64;
    assert_eq!(search.tried, 4 * words + 5 * words * words);

    let mut found = names_of(&search);
    found.sort_unstable();
    let mut expected = [
        "QuillGrommet",
        "_kestrelQuill",
        "GROMMET_KESTREL",
        "IQuill",
        "m_UQGrommet",
        "kestrelgrommet",
    ];
    expected.sort_unstable();
    assert_eq!(found, expected);

    let program = search
        .found
        .iter()
        .find(|found| found.kind == Target::Program);
    let program = program.expect("the program is found");
    assert_eq!(program.hash32, folded32("kestrelgrommet"));
    assert_eq!(program.hash64, name_hash64(b"kestrelgrommet"));
    let ty = search
        .found
        .iter()
        .find(|found| found.name == "QuillGrommet");
    let ty = ty.expect("the type is found");
    assert_eq!(
        (ty.kind, ty.hash64, ty.hash32),
        (
            Target::Member,
            literal_hash64(b"QuillGrommet"),
            folded32("QuillGrommet")
        )
    );

    // Three words a name, each in a case of its own; a name of four is past what was asked for.
    let mut longer = Targets::default();
    member(&mut longer, "UQKestrelGrommet");
    member(&mut longer, "quill_uq_kestrel_grommet");
    let three = Budget {
        join: 3,
        candidates: 1 << 20,
    };
    let search = super::search(WORDS, &longer, three);
    assert_eq!(names_of(&search), ["UQKestrelGrommet"]);
    assert_eq!(
        search.tried,
        4 * words + 5 * words.pow(2) + 5 * words.pow(3)
    );
    let four = Budget {
        join: MAX_JOIN,
        candidates: 1 << 20,
    };
    assert_eq!(super::search(WORDS, &longer, four).found.len(), 2);
}

#[test]
fn a_search_stops_at_its_budget_and_says_so() {
    let targets = targets();
    let words = WORDS.len() as u64;
    // Enough for every word alone and for no pair.
    let budget = Budget {
        join: 2,
        candidates: 4 * words,
    };
    let search = search(WORDS, &targets, budget);
    assert!(!search.complete);
    assert_eq!(search.tried, 4 * words);
    assert_eq!(names_of(&search), ["IQuill"]);

    // One word a name is all that was asked for: that is complete.
    let alone = Budget {
        join: 1,
        candidates: 4 * words,
    };
    let search = super::search(WORDS, &targets, alone);
    assert!(search.complete);
    assert_eq!(names_of(&search), ["IQuill"]);

    // More words a name than are ever joined are no more than the most.
    let deep = Budget {
        join: MAX_JOIN + 3,
        candidates: u64::MAX,
    };
    let search = super::search(["Quill"], &targets, deep);
    assert!(search.complete);
    assert_eq!(search.tried, 4 + 5 * (MAX_JOIN as u64 - 1));

    // Nothing sought, or no word, is nothing tried.
    let none = super::search(WORDS, &Targets::default(), Budget::default());
    assert_eq!((none.tried, none.complete), (0, true));
    let wordless = super::search(["", "two words", "é"], &targets, Budget::default());
    assert_eq!((wordless.tried, wordless.complete), (0, true));
}

#[test]
fn what_a_search_found_is_a_names_list_that_proves_itself() {
    let search = search(WORDS, &targets(), Budget::default());
    let list = search.list();
    assert!(list.starts_with("# "), "{list}");

    let mut names = Names::default();
    assert_eq!(
        names.read_list("found.tsv", &list),
        Ok(2 * search.found.len())
    );
    assert_eq!(names.rows, [(search.found.len(), 0)], "no row is on trust");
    assert_eq!(
        names.type_name(literal_hash64(b"QuillGrommet")),
        Some("QuillGrommet")
    );
    let owner = TypeKey(0xA0);
    assert!(names.bind_field(literal_hash64(b"_kestrelQuill"), owner, 8));
    assert_eq!(names.field(owner, 8), Some("_kestrelQuill"));
    let program = "kestrelgrommet";
    assert_eq!(names.program(u64::from(folded32(program))), Some(program));
    assert_eq!(
        names.program(name_hash64(program.as_bytes())),
        Some(program)
    );
}

#[test]
fn the_types_and_fields_of_a_program_no_list_names_are_what_is_sought() {
    let field = |name: &str, offset: u32| FieldRow {
        offset,
        name: literal_hash64(name.as_bytes()),
        name32: folded32(name),
        ..FieldRow::default()
    };
    let ty = |name: &str, fields: Vec<FieldRow>| TypeRow {
        name: literal_hash64(name.as_bytes()),
        name32: folded32(name),
        fields,
        ..TypeRow::default()
    };
    let program = Program {
        types: vec![
            ty("QuillGrommet", vec![field("_kestrelQuill", 8)]),
            ty(
                "Thimble",
                vec![field("_kestrelQuill", 16), field("m_UQGrommet", 24)],
            ),
            // A record that states no hash of a name.
            TypeRow::default(),
        ],
        ..Program::default()
    };

    let mut all = Targets::default();
    all.of_program(&program, &Names::default());
    assert_eq!(all.len(), 4, "a name two records state is sought once");
    let mut found = names_of(&search(WORDS, &all, Budget::default()))
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    found.sort_unstable();
    assert_eq!(found, ["QuillGrommet", "_kestrelQuill", "m_UQGrommet"]);

    // What a list names already is not sought again.
    let mut names = Names::default();
    let listed = format!(
        "0x{:016X} QuillGrommet\n0x{:016X} m_UQGrommet\n",
        literal_hash64(b"QuillGrommet"),
        literal_hash64(b"m_UQGrommet")
    );
    assert_eq!(names.read_list("listed", &listed), Ok(2));
    let mut rest = Targets::default();
    rest.of_program(&program, &names);
    assert_eq!(rest.len(), 2);
    assert!(!rest.is_empty() && Targets::default().is_empty());
}

#[test]
fn a_search_on_threads_finds_and_tries_what_one_thread_does() {
    let targets = targets();
    for join in 1..=MAX_JOIN {
        let budget = Budget {
            join,
            candidates: 1 << 20,
        };
        let one = search(WORDS, &targets, budget);
        for threads in [1, 3, 8] {
            let many = search_threads(WORDS, &targets, budget, threads);
            assert_eq!(many, one, "{join} words, {threads} threads");
        }
    }
    // A length whose joins the budget does not cover is not started.
    let short = Budget {
        join: 2,
        candidates: 4 * WORDS.len() as u64 + 1,
    };
    let many = search_threads(WORDS, &targets, short, 4);
    assert!(!many.complete);
    assert_eq!(many.tried, 4 * WORDS.len() as u64);
    assert_eq!(names_of(&many), ["IQuill"]);
    let none = search_threads(WORDS, &Targets::default(), Budget::default(), 4);
    assert_eq!((none.tried, none.complete, none.found.len()), (0, true, 0));
}

#[test]
fn a_name_splits_into_the_words_it_is_written_in() {
    let cases: [(&str, &[&str]); 7] = [
        ("QuillGrommet", &["Quill", "Grommet"]),
        ("_kestrelQuill", &["kestrel", "Quill"]),
        ("GROMMET_KESTREL", &["GROMMET", "KESTREL"]),
        ("m_UQGrommet", &["m", "UQ", "Grommet"]),
        ("Quill3Grommet", &["Quill", "3", "Grommet"]),
        ("kestrelgrommet", &["kestrelgrommet"]),
        ("__", &[]),
    ];
    for (name, words) in cases {
        assert_eq!(words_of(name), words, "{name}");
    }
}

#[test]
fn a_generic_type_is_found_filled_with_names_and_with_generics_found_before() {
    let mut targets = Targets::default();
    let sought = [
        "System.Collections.Generic.Dictionary`2+Enumerator<QuillGrommet, System.Guid>",
        "System.Collections.Generic.Dictionary`2<int, QuillGrommet>",
        "System.Collections.Generic.IEnumerable`1<QuillGrommet>",
        "System.Collections.Generic.KeyValuePair`2<QuillGrommet, bool>",
        // Its argument is found in the round before.
        "System.Nullable`1<System.Collections.Generic.KeyValuePair`2<QuillGrommet, bool>>",
    ];
    for name in sought {
        member(&mut targets, name);
    }
    // No name of the list, and a 64-bit hash of the name in lower case, which is no record's.
    member(&mut targets, "System.Collections.Generic.IList`1<Thimble>");
    let lowered = "System.Collections.Generic.IList`1<QuillGrommet>";
    let wrong = literal_hash64(lowered.to_ascii_lowercase().as_bytes());
    targets.member(wrong, folded32(lowered));
    let search = composed(["QuillGrommet"], &targets);
    let mut got = names_of(&search);
    got.sort_unstable();
    assert_eq!(got, sought);
    assert!(search.tried > 0 && search.complete);
    assert!(composed(["QuillGrommet"], &Targets::default())
        .found
        .is_empty());
}
