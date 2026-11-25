use super::*;
use crate::gta6::hash::{name_hash32, name_hash64};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::sample;

fn named(name: &str) -> NativeName {
    NativeName {
        name: name.to_owned(),
        ..NativeName::default()
    }
}

#[test]
fn a_list_of_hash_and_name_lines_names_types_and_programs() {
    let mut names = Names::default();
    let text = "\u{feff}# a comment\n\
                // another\n\
                \n\
                0x0A0B0C0D shared\n\
                0x0123456789ABCDEF CDemoThing   trailing words are not read\n\
                0X00000000000000CC demo_module\r\n";
    assert_eq!(names.read_list("a test's", text), Ok(3));
    assert_eq!(names.lists, ["a test's"]);

    assert_eq!(names.program(0x0A0B_0C0D), Some("shared"));
    assert_eq!(
        names.type_name(0x0A0B_0C0D),
        None,
        "eight digits are a program hash, which no type has"
    );
    assert_eq!(
        names.listed_type("type_0123456789ABCDEF"),
        Some("CDemoThing")
    );
    assert_eq!(
        names.listed_module("module_0x00000000000000CC"),
        Some("demo_module")
    );
    assert_eq!(
        names.listed_type("type_00000000000000CC"),
        Some("demo_module")
    );
    assert_eq!(names.listed_type("type_0123456789ABCDE"), None);
    assert_eq!(names.listed_module("module_0x0123"), None);
}

#[test]
fn a_table_names_every_leading_hash_of_a_row_and_skips_a_row_with_no_name() {
    let mut names = Names::default();
    let text = "program_hash\tfile_hash64\tname\tevidence\n\
                0x00000011\t0x0000000000000022\t\t\t0x0000000000000022\n\
                0x00000033\t0x0000000000000044\tdemo.program\ttwo words of evidence\n";
    assert_eq!(names.read_list("table.tsv", text), Ok(2));
    assert_eq!(names.program(0x33), Some("demo.program"));
    assert_eq!(names.program(0x44), Some("demo.program"));
    assert_eq!(names.type_name(0x44), Some("demo.program"));
    assert_eq!(names.program(0x11), None);
    assert_eq!(names.program(0x22), None);

    // The same rows again name nothing new, and are no conflict.
    assert_eq!(names.read_list("table.tsv again", text), Ok(0));
    assert_eq!(names.lists.len(), 2);

    // A program's hash folds case: the name in another case is the same name, and the first stays.
    assert_eq!(names.read_list("cased", "0x33 Demo.Program\n"), Ok(0));
    assert_eq!(names.program(0x33), Some("demo.program"));
}

#[test]
fn a_list_that_is_not_one_is_refused_with_its_line() {
    let line = |text: &str| match Names::default().read_list("bad", text) {
        Err(NamesError::Line { line, what, .. }) => (line, what),
        other => panic!("`{text}` read as {other:?}"),
    };
    // Only the first row may be column names.
    assert_eq!(
        line("hash name\n0x11 a\nnot a hash\n"),
        (3, "the line starts with no `0x` hash".to_owned())
    );
    assert_eq!(
        line("0x11 a\n0x11 b\n"),
        (2, "0x11 is named `b` here and `a` before".to_owned())
    );
    assert_eq!(line("0x11 a;b\n"), (1, "`a;b` is not a name".to_owned()));
    assert_eq!(
        line("0x11 a\n0x11223344556677889 b\n").0,
        2,
        "seventeen digits are no hash"
    );

    let mut names = Names::default();
    for nothing in ["", "# only a comment\n", "hash name\n", "0x11\n"] {
        assert_eq!(
            names.read_list("empty", nothing),
            Err(NamesError::Empty {
                label: "empty".to_owned()
            }),
            "`{nothing}`"
        );
    }
    assert!(
        names.lists.is_empty(),
        "a refused list is not one that named things"
    );
}

#[test]
fn the_natives_list_naming_most_of_a_listings_hashes_is_taken() {
    let one = NativeDb::from_names("one", [(0xA1, named("A"))]);
    let two = NativeDb::from_names("two", [(0xA1, named("A")), (0xB2, named("B"))]);
    let other = NativeDb::from_names("other", [(0xC3, named("C")), (0xD4, named("D"))]);
    let hashes: BTreeSet<u64> = [0xA1, 0xB2, 0xE5].into();

    let lists = [one.clone(), two.clone(), other.clone()];
    let best = Names::best_natives(&lists, &hashes).map(|(db, n)| (db.label(), n));
    assert_eq!(best, Some(("two", 2)));

    let tied = [one.clone(), one.clone()];
    let first = Names::best_natives(&tied, &hashes).map(|(db, n)| (std::ptr::eq(db, &tied[0]), n));
    assert_eq!(first, Some((true, 1)), "a tie goes to the list given first");

    // A list that names none of the listing's natives is not an answer.
    assert!(Names::best_natives(&[other], &hashes).is_none());
    assert!(Names::best_natives(&[], &hashes).is_none());
}

#[test]
fn a_listings_declarations_say_which_natives_it_names_itself() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let natives = declared_natives(listing.declarations());
    let expected: BTreeMap<u64, bool> = [
        (0xBB, false),
        (0x1111_2222_3333_4444, true),
        (0x5555_6666_7777_8888, true),
    ]
    .into();
    assert_eq!(natives, expected);
}

fn folded32(name: &str) -> u32 {
    literal_string_hash(name.to_ascii_lowercase().as_bytes())
}

fn kept32(name: &str) -> u32 {
    literal_string_hash(name.as_bytes())
}

fn held<K: Eq + Hash>(map: &HashMap<K, String>, key: K) -> Option<&str> {
    map.get(&key).map(String::as_str)
}

/// The line a list is refused at, and why.
fn refused(text: &str) -> (usize, String) {
    match Names::default().read_list("bad", text) {
        Err(NamesError::Line { line, what, .. }) => (line, what),
        other => panic!("`{text}` read as {other:?}"),
    }
}

/// Two names of one 32-bit hash, found by trying: eighty thousand names hold such a pair as often as not.
fn two_names_of_one_hash() -> (u32, String, String) {
    let mut seen: HashMap<u32, String> = HashMap::new();
    for n in 0..1_000_000 {
        let name = format!("name{n}");
        let hash = kept32(&name);
        if let Some(first) = seen.insert(hash, name.clone()) {
            return (hash, first, name);
        }
    }
    panic!("a million names held no two of one hash");
}

#[test]
fn a_row_whose_hash_is_a_hash_of_its_name_names_what_that_function_names() {
    let program = folded32("Demo_Program");
    let method = kept32("GetOwner");
    let ty = murmur_low(b"DemoType");
    let module = name_hash64(b"Demo_Module");
    let ctor = kept32(".ctor");
    let text = format!(
        "0x{program:08X} Demo_Program\n0x{method:08X} GetOwner\n0x{ty:016X} DemoType\n\
         0x{module:016X} Demo_Module\n0x{ctor:08X} .ctor\n"
    );
    let mut names = Names::default();
    assert_eq!(names.read_list("proved.tsv", &text), Ok(5));
    assert_eq!(names.rows, [(5, 0)]);
    assert_eq!(
        names.sources(),
        ["`proved.tsv` (5 row(s) proved by their hash, 0 on trust)"]
    );

    assert_eq!(names.program(u64::from(program)), Some("Demo_Program"));
    assert_eq!(held(&names.texts, program), Some("Demo_Program"));
    assert_eq!(held(&names.method_names, program), None);

    // A name spelled with capitals is not the hash of its lower case: a method's row names no program.
    assert_eq!(held(&names.method_names, method), Some("GetOwner"));
    assert_eq!(names.program(u64::from(method)), None);
    assert_eq!(held(&names.texts, method), None);

    assert_eq!(names.type_name(ty), Some("DemoType"));
    assert_eq!(held(&names.field_names, ty), Some("DemoType"));
    assert_eq!(
        names.program(ty),
        None,
        "a type's hash keeps case, a module's folds it"
    );

    assert_eq!(names.program(module), Some("Demo_Module"));
    assert_eq!(held(&names.texts64, module), Some("Demo_Module"));
    assert_eq!(names.type_name(module), None);

    // `.ctor` has no capital: one hash by both functions, so a method's name and a text.
    assert_eq!(held(&names.method_names, ctor), Some(".ctor"));
    assert_eq!(held(&names.texts, ctor), Some(".ctor"));
}

#[test]
fn a_row_proved_by_one_column_and_not_by_another_is_refused() {
    let (short, long) = (kept32("demo"), name_hash64(b"demo"));
    let both = format!("0x{short:08X}\t0x{long:016X}\tdemo\n");
    assert_eq!(Names::default().read_list("both", &both), Ok(2));

    let (line, what) = refused(&format!("0x{short:08X}\t0x{:016X}\tdemo\n", long ^ 1));
    assert_eq!(line, 1);
    assert!(what.contains("some of the row's hash columns"), "{what}");

    // A text may hold what no name does, where its hash proves it.
    let text = "a name, with spaces";
    let proved = format!("hash\tname\n0x{:08X}\t{text}\n", kept32(text));
    let mut names = Names::default();
    assert_eq!(names.read_list("text", &proved), Ok(1));
    assert_eq!(held(&names.texts, kept32(text)), Some(text));
    assert!(
        names.programs.is_empty(),
        "a program prints under its name in code"
    );
    // The same words under a hash that is not theirs are no label.
    assert_eq!(
        refused(&format!("hash\tname\n0x00000011\t{text}\n")),
        (2, format!("`{text}` is not a name"))
    );
}

#[test]
fn a_hash_two_names_prove_themselves_for_names_nothing() {
    let (hash, first, second) = two_names_of_one_hash();
    let wide = u64::from(hash);
    let mut names = Names::default();
    let text = format!("0x{hash:08X} {first}\n0x{hash:08X} {second}\n0x{hash:08X} {first}\n");
    assert_eq!(names.read_list("two", &text), Ok(1));
    assert_eq!(held(&names.texts, hash), None);
    assert_eq!(names.program(wide), None);
    assert!(names.method_names.is_empty());
    assert!(names.ambiguous.contains(&(Proof::Folded32, wide)));

    // One name twice, in another case, is one name.
    let mut names = Names::default();
    let shouted = first.to_ascii_uppercase();
    let text = format!("0x{hash:08X} {first}\n0x{hash:08X} {shouted}\n");
    assert_eq!(names.read_list("one", &text), Ok(1));
    assert_eq!(held(&names.texts, hash), Some(first.as_str()));
    assert!(names.ambiguous.is_empty());

    // A label given on trust is the user's word: a name proved against it is an error.
    let mut names = Names::default();
    let label = format!("0x{hash:08X} my_label\n");
    assert_eq!(names.read_list("label", &label), Ok(1));
    let proved = names.read_list("proved", &format!("0x{hash:08X} {first}\n"));
    assert!(
        matches!(proved, Err(NamesError::Line { line: 1, .. })),
        "{proved:?}"
    );
}

#[test]
fn a_table_of_texts_names_no_program_and_takes_nothing_on_trust() {
    let (hash, wide) = (kept32("demo_text"), name_hash64(b"demo/asset"));
    let mut names = Names::default();
    let short = format!("hash32\ttext\tproof\n0x{hash:08X}\tdemo_text\tvalue-only\n");
    assert_eq!(names.read_list("texts", &short), Ok(1));
    let long = format!("hash64\ttext\n0x{wide:016X}\tDemo/Asset\n");
    assert_eq!(names.read_list("wide", &long), Ok(1));
    assert_eq!(held(&names.texts, hash), Some("demo_text"));
    assert_eq!(held(&names.texts64, wide), Some("Demo/Asset"));
    assert!(
        names.programs.is_empty() && names.method_names.is_empty() && names.types.is_empty(),
        "under `text` a row is the text of a constant and nothing else"
    );

    // The same row under `name` names a program and a method too.
    let mut named = Names::default();
    let row = format!("hash\tname\n0x{hash:08X}\tdemo_text\n");
    assert_eq!(named.read_list("names", &row), Ok(1));
    assert_eq!(named.program(u64::from(hash)), Some("demo_text"));
    assert_eq!(held(&named.method_names, hash), Some("demo_text"));

    // No text is taken on trust.
    let other = format!("hash32\ttext\n0x{:08X}\tdemo_text\n", hash ^ 1);
    let why = "`demo_text` is not the text the row's hash is of";
    assert_eq!(refused(&other), (2, why.to_owned()));
}

#[test]
fn a_table_of_cells_fields_or_builtins_reads_by_its_column_names() {
    let mut names = Names::default();
    let slots = "native_hash64\tcell\tkind\tnative\n0x00000000000000D1\t2\tHash\tDEMO.SET\n";
    assert_eq!(names.read_list("slots", slots), Ok(1));
    assert!(names.hash_slots.contains(&(0xD1, 2)));

    let fields = "owner_type_hash64\toffset\tname\n\
                  0x00000000000000A0\t0x10\t_tint\n\
                  0x00000000000000A0\t0x18\t\n";
    assert_eq!(names.read_list("fields", fields), Ok(1));
    assert_eq!(names.field(TypeKey(0xA0), 0x10), Some("_tint"));
    assert_eq!(names.field(TypeKey(0xA0), 0x18), None);

    let members = "owner_type_hash64\tslot\tname\n\
                   0x00000000000000A0\t4\tMoveNext\n\
                   0x00000000000000A0\t5\t\n";
    assert_eq!(names.read_list("members", members), Ok(1));
    assert_eq!(names.method(TypeKey(0xA0), 4), Some("MoveNext"));
    assert_eq!(names.method(TypeKey(0xA0), 5), None);

    let builtins = "type_index\tmethod_index\twrapper_path\tarity\n4\t2\tDemo.List.get_Count\t1\n";
    assert_eq!(names.read_list("builtins", builtins), Ok(1));
    assert_eq!(names.builtin(4, 2), Some("Demo.List.get_Count"));
    assert_eq!(names.builtin(2, 4), None);

    let types = "index\tname\tbyte_size\n11\tDemo.Int\t4\n16\t\t\n";
    assert_eq!(names.read_list("types", types), Ok(1));
    assert_eq!(names.listed_type("type_8000000B"), Some("Demo.Int"));
    assert_eq!(names.listed_type("type_0000000B"), None);

    assert!(
        names.programs.is_empty() && names.texts.is_empty(),
        "none of them is a row of hashes and a name"
    );
    assert_eq!(names.rows, [(0, 1); 5]);

    // A cell a list states is a hash's, and is stated by a number.
    assert_eq!(refused("native_hash64\tcell\tkind\n0xD1\t2\tfloat\n").0, 2);
    assert_eq!(refused("native_hash64\tcell\tkind\n0xD1\ttwo\tHash\n").0, 2);
    // Native names are a natives list's.
    assert_eq!(
        refused("native_hash64\tname\tnamespace\n0xD1\tSET\tDEMO\n").0,
        1
    );
    assert_eq!(
        refused("owner_type_hash64\toffset\tname\n0xA0\t0x10\ttwo words\n"),
        (2, "`two words` is not a name".to_owned())
    );
    assert_eq!(
        refused("owner_type_hash64\tslot\tname\n0xA0\tfour\tMoveNext\n").0,
        2
    );
    assert_eq!(
        refused("index\tname\n11\tDemo.Int\n11\tDemo.Other\n"),
        (3, "`Demo.Other` here was `Demo.Int` before".to_owned())
    );
}

#[test]
fn a_method_or_field_record_binds_the_name_a_list_holds_for_its_hash() {
    let (start, ctor, tint) = (kept32("Start"), kept32(".ctor"), murmur_low(b"_tint"));
    let mut names = Names::default();
    let text = format!("0x{start:08X} Start\n0x{ctor:08X} .ctor\n0x{tint:016X} _tint\n");
    assert_eq!(names.read_list("members", &text), Ok(3));

    let owner = TypeKey(0xA0);
    assert!(names.bind_method(start, owner, 3));
    assert_eq!(names.method(owner, 3), Some("Start"));
    assert_eq!(
        names.method(TypeKey(0xA1), 3),
        None,
        "a slot's name is its owner's"
    );
    assert!(
        !names.bind_method(ctor, owner, 0),
        "`.ctor` is no name a call is written with"
    );
    assert!(!names.bind_method(start ^ 1, owner, 4));
    assert_eq!(names.methods.len(), 1);

    assert!(names.bind_field(tint, owner, 0x10));
    assert_eq!(names.field(owner, 0x10), Some("_tint"));
    assert!(!names.bind_field(tint ^ 1, owner, 0x18));
}

#[test]
fn a_text_is_for_a_number_large_enough_to_be_a_hash() {
    let mut names = Names::default();
    let short = [0xFFFF, 0x1_0000, 0xFFFE_FFFF, 0xFFFF_0000];
    let long = [0x1_0000_0000, 0x8A2B_3C4D, 0xFFFF_FFFF_8A2B_3C4D];
    names.texts.extend(short.map(|hash| (hash, "t".to_owned())));
    names
        .texts64
        .extend(long.map(|hash| (hash, "t".to_owned())));
    assert_eq!(
        short.map(|hash| names.text(hash).is_some()),
        [false, true, true, false]
    );
    assert_eq!(
        long.map(|hash| names.text64(hash).is_some()),
        [true, false, false]
    );
}

#[test]
fn the_hash_that_keeps_case_is_the_one_that_folds_it_over_lower_case() {
    for text in [
        "",
        "a",
        "demo_module",
        "exactly sixteen!",
        "seventeen letters",
        "the quick brown fox jumps over the lazy dog",
    ] {
        assert_eq!(
            murmur_low(text.as_bytes()),
            name_hash64(text.as_bytes()),
            "`{text}`"
        );
    }
    assert_ne!(murmur_low(b"Demo"), name_hash64(b"Demo"));
    assert_eq!(murmur_low(b"demo"), name_hash64(b"Demo"));
    assert_eq!(folded32("Demo"), name_hash32(b"Demo"));
    assert_ne!(kept32("Demo"), name_hash32(b"Demo"));
}

#[test]
fn a_table_of_hash_and_rva_gives_a_native_its_hash_by_its_handlers_address() {
    let mut names = Names::default();
    let table = "hash\trva\tnamespace\tname\n0x00000000000000AA\t0x01020304\tDEMO\tGET\n";
    assert_eq!(names.read_list("addresses", table), Ok(1));
    assert_eq!(names.native_at(0x0102_0304), Some(0xAA));
    assert_eq!(names.native_at(0x0102_0308), None);
    // ⚠ The table names nothing: a natives list names the hash it gives.
    assert!(names.programs.is_empty() && names.types.is_empty());

    // One handler is one native's.
    let twice = "hash\trva\n0x00000000000000AA\t0x01020304\n0x00000000000000BB\t0x01020304\n";
    let refused = Names::default().read_list("addresses", twice);
    assert!(
        matches!(refused, Err(NamesError::Line { line: 3, .. })),
        "{refused:?}"
    );
}
