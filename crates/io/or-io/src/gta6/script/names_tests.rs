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

#[test]
fn a_list_of_words_names_only_what_a_words_own_hashes_are_found_to_be() {
    let mut names = Names::default();
    let text = "# words, one a line\nQuillGrommet\n\nkestrel_vexa\nGetThimble\n_sprocket\n\
                kestrel_vexa\n";
    assert_eq!(names.read_list("words.txt", text), Ok(4));
    assert_eq!(names.candidates, [5]);
    assert_eq!(
        names.sources(),
        ["`words.txt` (5 word(s) with no hash, each a name only of what its own hashes are found \
          to be)"]
    );
    // A word is no statement: it stands in none of the maps a row with a hash fills.
    assert!(names.types.is_empty() && names.programs.is_empty() && names.texts.is_empty());
    assert!(names.method_names.is_empty() && names.field_names.is_empty());

    // A type and a field by the 64 bits of the name as it is spelled.
    let ty = murmur_low(b"QuillGrommet");
    assert_eq!(names.type_name(ty), Some("QuillGrommet"));
    assert_eq!(
        names.listed_type(&format!("type_{ty:016X}")),
        Some("QuillGrommet")
    );
    assert_eq!(names.type_name(murmur_low(b"quillgrommet")), None);
    let owner = TypeKey(0xA0);
    assert!(names.bind_field(murmur_low(b"_sprocket"), owner, 0x10));
    assert_eq!(names.field(owner, 0x10), Some("_sprocket"));

    // A module by the 64 bits of the name in lower case, and so the text of a 64-bit constant.
    let file = name_hash64(b"kestrel_vexa");
    assert_eq!(names.program(file), Some("kestrel_vexa"));
    assert_eq!(names.text64(file), Some("kestrel_vexa"));

    // ⚠ A program by both of its hashes and never by its 32 bits alone.
    let program = folded32("kestrel_vexa");
    assert_eq!(names.program(u64::from(program)), None);
    assert!(!names.bind_program(program ^ 1, file));
    assert!(!names.bind_program(program, file ^ 1));
    assert!(!names.bind_program(program, u64::from(program)));
    assert_eq!(names.program(u64::from(program)), None);
    assert!(names.bind_program(program, file));
    assert_eq!(names.program(u64::from(program)), Some("kestrel_vexa"));

    // The text of a 32-bit constant, which the names pass shows in a covered place alone.
    assert_eq!(names.text(program), Some("kestrel_vexa"));

    // A method by the 32 bits of its record, the list being short.
    let get = kept32("GetThimble");
    assert_eq!(names.method_name(folded32("GetThimble")), None);
    assert!(names.bind_method(get, owner, 3));
    assert_eq!(names.method(owner, 3), Some("GetThimble"));

    // A row with a hash is a statement, and goes before a word.
    names.method_names.insert(get, "Stated".to_owned());
    assert_eq!(names.method_name(get), Some("Stated"));
    names.types.insert(ty, "Labelled".to_owned());
    assert_eq!(names.type_name(ty), Some("Labelled"));
    // And a hash two rows proved different names for is no word's either.
    names
        .ambiguous
        .insert((Proof::Kept64, murmur_low(b"_sprocket")));
    assert!(!names.bind_field(murmur_low(b"_sprocket"), owner, 0x18));
}

#[test]
fn a_file_is_a_list_of_words_only_where_every_line_of_it_is_one_word() {
    // The first row decides, and a row of column names has two columns or more.
    assert_eq!(Names::default().read_list("one", "zorb\n"), Ok(1));
    assert_eq!(
        refused("zorb\n0x11 a\n"),
        (2, NOT_A_WORD.to_owned()),
        "a table under one word of a title is not read as words"
    );
    assert_eq!(refused("zorb\ntwo words\n"), (2, NOT_A_WORD.to_owned()));
    assert_eq!(refused("zorb\n},\n"), (2, NOT_A_WORD.to_owned()));
    // What a natives list kept in the same folder opens with is no word, so no list of them.
    assert_eq!(
        refused("{\n\"DEMO\": {}\n}\n"),
        (2, "the line starts with no `0x` hash".to_owned())
    );
    // A hash alone on the first line is a row with no name, as it was.
    let empty = Names::default().read_list("empty", "0x11\n");
    assert!(matches!(empty, Err(NamesError::Empty { .. })), "{empty:?}");

    // The same word twice is one word, and another spelling of it another.
    let mut names = Names::default();
    assert_eq!(names.read_list("thrice", "zorb\nzorb\nZorb\n"), Ok(2));
    assert_eq!(names.words.len(), 2);
    // A copy of the names made to try a list on shares the words until a list of words is read.
    let mut tried = names.clone();
    assert!(Arc::ptr_eq(&tried.words, &names.words));
    assert_eq!(tried.read_list("table", "0x11 a\n"), Ok(1));
    assert!(Arc::ptr_eq(&tried.words, &names.words));
    assert_eq!(tried.candidates, [3, 0]);
    assert_eq!(tried.read_list("more", "vexa\n"), Ok(1));
    assert_eq!((tried.words.len(), names.words.len()), (3, 2));
}

#[test]
fn the_vocabulary_kept_of_an_install_reads_as_words() {
    let mut kept = crate::gta6::names::Vocabulary::default();
    assert!(kept.add("Kestrel_Vexa", None));
    assert!(kept.add("zorb", Some(*b"dr")));
    let mut written = Vec::new();
    kept.write(&mut written).expect("it writes to memory");
    let text = String::from_utf8(written).expect("names are text");

    let mut names = Names::default();
    assert_eq!(names.read_list("vocabulary.txt", &text), Ok(2));
    assert_eq!(names.candidates, [2]);
    // The vocabulary keeps a name in lower case, which is what a module's hash is of.
    assert_eq!(
        names.program(name_hash64(b"Kestrel_Vexa")),
        Some("kestrel_vexa")
    );
    assert_eq!(names.program(name_hash64(b"zorb")), Some("zorb"));
    assert_eq!(names.type_name(murmur_low(b"Kestrel_Vexa")), None);

    // Its first line alone names nothing, and a line that is no name is passed over.
    let head = text.lines().next().expect("it has a first line");
    let alone = Names::default().read_list("empty", head);
    assert!(matches!(alone, Err(NamesError::Empty { .. })), "{alone:?}");
    let odd = format!("{head}\nnot a line of it at all\n-- sprocket\n");
    assert_eq!(Names::default().read_list("odd", &odd), Ok(1));
}

#[test]
fn a_list_of_more_words_than_32_bits_can_name_by_names_no_method() {
    let few: String = (0..words::METHOD_WORDS)
        .map(|n| format!("w{n}\n"))
        .collect();
    let mut names = Names::default();
    assert_eq!(names.read_list("few", &few), Ok(words::METHOD_WORDS));
    let owner = TypeKey(0xA0);
    assert!(names.bind_method(kept32("w7"), owner, 1));

    assert_eq!(names.read_list("one more", "w_more\n"), Ok(1));
    assert!(!names.bind_method(kept32("w8"), owner, 2));
    assert_eq!(names.method(owner, 2), None);
    // By 64 bits a list of any length names.
    assert_eq!(names.type_name(murmur_low(b"w8")), Some("w8"));
}

#[test]
fn a_proved_runtime_type_name_is_written_in_the_spelling_of_csharp() {
    let names_of = [
        ("Quill+Grommet", Some("Quill.Grommet")),
        (
            "Demo.Quill`2<Sprocket, int>",
            Some("Demo.Quill<Sprocket, int>"),
        ),
        // A nested type's arguments are those of the generic type it is in.
        (
            "Quill`2+Grommet<Sprocket, int>",
            Some("Quill<Sprocket, int>.Grommet"),
        ),
        (
            "Quill`1+Grommet`1<Quill`1<int>, bool>",
            Some("Quill<Quill<int>>.Grommet<bool>"),
        ),
        (
            "Quill`2<int[], Sprocket[][]>",
            Some("Quill<int[], Sprocket[][]>"),
        ),
        // A generic type itself, arguments its types do not take, and any other mark: unwritten.
        ("Quill`2", None),
        ("Quill`2<int>", None),
        ("Quill<int>", None),
        ("<Sprocket>+Grommet", None),
        ("Quill+Grommet=4", None),
    ];
    let rows: String = names_of
        .iter()
        .map(|(name, _)| format!("0x{:016X}\t{name}\n", murmur_low(name.as_bytes())))
        .collect();
    let mut names = Names::default();
    assert_eq!(names.read_list("types.tsv", &rows), Ok(names_of.len()));
    for (name, shown) in names_of {
        assert_eq!(
            names.type_name(murmur_low(name.as_bytes())),
            shown,
            "{name}"
        );
    }
    // Arguments nested past any name a list holds are no name.
    let deep = format!("{}int{}", "Quill`1<".repeat(40), ">".repeat(40));
    let row = format!("0x{:016X}\t{deep}\n", murmur_low(deep.as_bytes()));
    assert_eq!(names.read_list("deep.tsv", &row), Ok(1));
    assert_eq!(names.type_name(murmur_low(deep.as_bytes())), None);
}

#[test]
fn a_row_of_a_program_and_an_offset_names_that_function() {
    let mut names = Names::default();
    let text = "0x0A0B0C0D:0x614E09 Demo_Run\n\
                ZorbleTown:func_000100 SpinUp\n\
                program_0000ABCD:0X20 Snoozle\n\
                zorbletown:0x200\n";
    assert_eq!(names.read_list("functions.txt", text), Ok(3));
    assert_eq!(names.rows, [(0, 3)], "a function's name is taken on trust");
    let town = folded32("zorbletown");
    assert_eq!(
        names.function(FunctionKey::new(0x0A0B_0C0D, 0x0061_4E09)),
        Some("Demo_Run")
    );
    assert_eq!(
        names.function(FunctionKey::new(town, 0x100)),
        Some("SpinUp"),
        "a program by its name is the program of that name's hash, in any case"
    );
    assert_eq!(
        names.function(FunctionKey::new(0xABCD, 0x20)),
        Some("Snoozle")
    );
    assert_eq!(names.function(FunctionKey::new(town, 0x200)), None);
    assert_eq!(
        names.program(u64::from(town)),
        None,
        "a function's row names no program"
    );

    // The same rows again name nothing new.
    assert_eq!(names.read_list("functions.txt again", text), Ok(0));

    let p = folded32("p");
    assert_eq!(
        refused("p:0x10 Plinked\np:0x10 Wobbling\n"),
        (
            2,
            format!("the function at 0x{p:08X}:0x10 is named `Wobbling` here and `Plinked` before")
        )
    );
    assert_eq!(
        refused("p:0x10 demo.run\n"),
        (1, "`demo.run` is not a name".to_owned())
    );
    // Sixteen digits are a module's hash, and nine an offset no program has.
    for line in ["0x0000000000000011:0x10 Wobbling", "p:0x123456789 Wobbling"] {
        let text = format!("0x11:0x10 Plinked\n{line}\n");
        assert_eq!(
            refused(&text),
            (2, "the line starts with no `0x` hash".to_owned()),
            "{line}"
        );
    }
}

#[test]
fn an_entry_the_container_states_by_hash_is_named_by_the_text_a_row_proves() {
    let (plaza, dock) = (folded32("zorbleplaza"), folded32("zorbledock"));
    let (spaced, quay) = (folded32("zorble plaza"), folded32("zorblequay"));
    let mut names = Names::default();
    let text = format!(
        "hash32\ttext\n0x{plaza:08X}\tZorblePlaza\n0x{dock:08X}\tZorbleDock\n\
         0x{spaced:08X}\tzorble plaza\n"
    );
    assert_eq!(names.read_list("entries.tsv", &text), Ok(3));
    // A word is no proof of a 32-bit hash.
    assert_eq!(names.read_list("words.txt", "zorblequay\n"), Ok(1));
    assert_eq!(names.words.folded32(quay), Some("zorblequay"));

    let entry = |name: u32, at: u32| EntryRow { name, at };
    let program = 0x5566_7788;
    let entries = [
        entry(plaza, 0x40),
        entry(spaced, 0x80),
        entry(quay, 0xC0),
        entry(0, 0xE0),
        entry(0x1234_5678, 0x100),
    ];
    assert_eq!(names.bind_entries(program, &entries), 1);
    assert_eq!(
        names.function(FunctionKey::new(program, 0x40)),
        Some("ZorblePlaza")
    );
    for at in [0x80, 0xC0, 0xE0, 0x100] {
        assert_eq!(
            names.function(FunctionKey::new(program, at)),
            None,
            "{at:X}"
        );
    }
    assert_eq!(names.bind_entries(program, &entries), 0, "nothing new");

    // A name the user gave the function stays.
    let mut mine = names.clone();
    mine.functions
        .insert(FunctionKey::new(0x99, 0x40), "Mine".to_owned());
    assert_eq!(mine.bind_entries(0x99, &[entry(plaza, 0x40)]), 0);
    assert_eq!(mine.function(FunctionKey::new(0x99, 0x40)), Some("Mine"));

    // One hash stated at two functions, and two at one function, name neither.
    let mut apart = names.clone();
    let twice = [entry(plaza, 0x10), entry(plaza, 0x20)];
    assert_eq!(apart.bind_entries(0xAA, &twice), 0);
    let both = [entry(plaza, 0x10), entry(dock, 0x10)];
    assert_eq!(apart.bind_entries(0xBB, &both), 0);
    assert_eq!(apart.bind_entries(0xCC, &[entry(dock, 0x10)]), 1);
    assert_eq!(
        apart.function(FunctionKey::new(0xCC, 0x10)),
        Some("ZorbleDock")
    );
}

#[test]
fn a_table_of_enum_members_names_a_value_of_its_enum_under_either_reading_of_its_sign() {
    let mut names = Names::default();
    let (e1, e2) = (TypeKey(0xE1), TypeKey(0xE2));
    assert_eq!(names.enum_member(e1, 0, 4), None, "no list, no member");
    let text = "enum_type_hash64\tmember\tvalue\n\
                0x00000000000000E1\tPlinked\t0\n\
                0x00000000000000E1\tWobbling\t1\n\
                0x00000000000000E1\tFrazzled\t-1\n\
                type_00000000000000E1\tTwinkled\t0x7F\n\
                0x00000000000000E1\tFlimsy\t0\n\
                0x00000000000000E2\tQuibble\t4294967295\n\
                0x00000000000000E2\tSnoozle\t-1\n\
                0x00000000000000E2\t\t9\n";
    assert_eq!(names.read_list("members.tsv", text), Ok(6));
    assert_eq!(names.rows, [(0, 7)]);

    for (value, bytes, member) in [
        (0, 4, Some("Plinked")),
        (1, 1, Some("Wobbling")),
        (0xFFFF_FFFF, 4, Some("Frazzled")),
        (0xFF, 1, Some("Frazzled")),
        (u64::MAX, 8, Some("Frazzled")),
        (0x7F, 1, Some("Twinkled")),
        (2, 4, None),
        // More than the enum's bytes hold, and no width an enum has.
        (0x1_0000_0000, 4, None),
        (0, 0, None),
        (0, 9, None),
    ] {
        assert_eq!(
            names.enum_member(e1, value, bytes),
            member,
            "{value:X} in {bytes}"
        );
    }
    // Two members of the two readings of one value's sign name neither; eight bytes have one reading.
    assert_eq!(names.enum_member(e2, 0xFFFF_FFFF, 4), None);
    assert_eq!(names.enum_member(e2, 0xFFFF_FFFF, 8), Some("Quibble"));
    assert_eq!(names.enum_member(TypeKey(0xE3), 0, 4), None);

    let head = "enum_type_hash64\tmember\tvalue\n";
    assert_eq!(
        refused(&format!("{head}0xE1\tPlinked\tten\n")),
        (
            2,
            "the row does not start as its column names say".to_owned()
        )
    );
    assert_eq!(
        refused(&format!("{head}0xE1\tdemo.member\t1\n")),
        (2, "`demo.member` is not a name".to_owned())
    );
}

#[test]
fn a_later_lists_name_of_a_builtin_stands_in_place_of_an_earlier_lists_and_the_header_says_so() {
    let head = "type_index\tmethod_index\twrapper_path\n";
    let mut names = Names::default();
    let first = format!("{head}4\t2\tDemo.Flask.Swirl\n4\t3\tDemo.Flask.Cork\n");
    assert_eq!(names.read_list("first.tsv", &first), Ok(2));
    let second = format!(
        "{head}4\t2\tDemo.Flask.SwirlInternal\n4\t3\tDemo.Flask.Cork\n4\t5\tDemo.Flask.Gurgle\n"
    );
    assert_eq!(
        names.read_list("second.tsv", &second),
        Ok(1),
        "a name in place of another is of a builtin a list named before"
    );
    assert_eq!(names.builtin(4, 2), Some("Demo.Flask.SwirlInternal"));
    assert_eq!(names.builtin(4, 3), Some("Demo.Flask.Cork"));
    assert_eq!(names.builtin(4, 5), Some("Demo.Flask.Gurgle"));

    let types = "index\tname\n11\tDemo.Flagon\n";
    assert_eq!(names.read_list("types.tsv", types), Ok(1));
    let again = "index\tname\n11\tDemo.Tankard\n";
    assert_eq!(names.read_list("types again.tsv", again), Ok(0));
    assert_eq!(names.listed_type("type_8000000B"), Some("Demo.Tankard"));

    // The first list's name again, after the second's, stands in place of the second's.
    let third = format!("{head}4\t2\tDemo.Flask.Swirl\n");
    assert_eq!(names.read_list("third.tsv", &third), Ok(0));
    assert_eq!(names.builtin(4, 2), Some("Demo.Flask.Swirl"));

    assert_eq!(
        names.sources(),
        [
            "`first.tsv` (0 row(s) proved by their hash, 2 on trust)",
            "`second.tsv` (0 row(s) proved by their hash, 3 on trust; 1 builtin name(s) in \
             place of `first.tsv`'s)",
            "`types.tsv` (0 row(s) proved by their hash, 1 on trust)",
            "`types again.tsv` (0 row(s) proved by their hash, 1 on trust; 1 builtin name(s) in \
             place of `types.tsv`'s)",
            "`third.tsv` (0 row(s) proved by their hash, 1 on trust; 1 builtin name(s) in place \
             of `second.tsv`'s)",
        ]
    );

    // One list still names one builtin once, and a type no builtin row named keeps its one label.
    assert_eq!(
        refused(&format!("{head}4\t2\tDemo.A\n4\t2\tDemo.B\n")),
        (3, "`Demo.B` here was `Demo.A` before".to_owned())
    );
    let mut labelled = Names::default();
    let one = "0x0123456789ABCDEF DemoFlagon\n";
    assert_eq!(labelled.read_list("one", one), Ok(1));
    let other = "0x0123456789ABCDEF DemoTankard\n";
    assert!(labelled.read_list("other", other).is_err());
}

#[test]
fn a_table_of_texts_of_an_enums_values_names_its_members() {
    let wink = folded32("zorble_wink");
    let blink = folded32("zorble/blink");
    let (zorble, other) = (TypeKey(0x00E1_0000_0000_00E1), TypeKey(0xE2));
    let mut names = Names::default();
    let text = format!(
        "hash32\ttext\tenum_type_hash64\tproof\n\
         0x{wink:08X}\tzorble_wink\ttype_00E10000000000E1\tsibling\n\
         0x{blink:08X}\tzorble/blink\t0x00E10000000000E1\tsibling\n"
    );
    assert_eq!(names.read_list("enum texts", &text), Ok(2));
    assert_eq!(held(&names.texts, wink), Some("zorble_wink"));
    assert!(names.programs.is_empty(), "a text names no program");
    let member = |names: &Names, ty: TypeKey, bytes: u32| {
        names
            .enum_member(ty, u64::from(wink), bytes)
            .map(str::to_owned)
    };
    assert_eq!(member(&names, zorble, 4).as_deref(), Some("zorble_wink"));
    assert_eq!(
        names.enum_member(zorble, u64::from(blink), 4),
        None,
        "a text that is no identifier names no member"
    );
    assert_eq!(
        member(&names, other, 4),
        None,
        "only the enum the row states"
    );
    assert_eq!(
        member(&names, zorble, 8),
        None,
        "a 32-bit hash is no value of 8 bytes"
    );

    // A row of members goes before a text.
    let members =
        format!("enum_type_hash64\tmember\tvalue\n0x00E10000000000E1\tWinked\t0x{wink:08X}\n");
    assert_eq!(names.read_list("members", &members), Ok(1));
    assert_eq!(member(&names, zorble, 4).as_deref(), Some("Winked"));

    // Under `text` alone a row states no enum.
    let mut plain = Names::default();
    let row = format!("hash32\ttext\n0x{wink:08X}\tzorble_wink\n");
    assert_eq!(plain.read_list("texts", &row), Ok(1));
    assert_eq!(member(&plain, zorble, 4), None);

    // A text is still proved by its hash, and the enum's column holds a type.
    let head = "hash32\ttext\tenum_type_hash64\n";
    let unproved = format!(
        "{head}0x{:08X}\tzorble_wink\ttype_00E10000000000E1\n",
        wink ^ 1
    );
    assert_eq!(refused(&unproved).0, 2);
    assert_eq!(
        refused(&format!("{head}0x{wink:08X}\tzorble_wink\tsomething\n")),
        (2, "`something` is no enum's type".to_owned())
    );
}

#[test]
fn a_list_of_labels_labels_only_what_no_natives_list_names_and_the_header_calls_them_guesses() {
    let mut names = Names::default();
    let text = "# guesses, not names\n\
                hash\trva\tlabel\tconfidence\n\
                0x00000000000000D1\t0x00001000\t_GET_DEMO_FLASK\tlow\n\
                0x00000000000000D2\t0x00002000\tDEMO::_SET_DEMO_FLASK\tmedium\n\
                0x00000000000000D3\t0x00003000\t\tlow\n\
                0x00000000000000D4\t0x00004000\t_0x00000000000000D4\tlow\n";
    assert_eq!(names.read_labels("labels.tsv", text), Ok(2));
    assert!(
        names.lists.is_empty() && names.native_hashes.is_empty(),
        "a list of labels is no names list, and states no handler"
    );
    assert!(
        names.native(0xD1).is_none(),
        "a label is no natives list's name"
    );
    let label = |names: &Names, hash: u64| names.native_label(hash).map(ToString::to_string);
    assert_eq!(label(&names, 0xD1).as_deref(), Some("_GET_DEMO_FLASK"));
    assert_eq!(
        label(&names, 0xD2).as_deref(),
        Some("DEMO::_SET_DEMO_FLASK")
    );
    assert_eq!(label(&names, 0xD3), None);
    assert_eq!(
        label(&names, 0xD4),
        None,
        "a label that is its hash labels nothing"
    );

    // A later list's label stands in place of an earlier one's.
    let more = "0x00000000000000D1 _GET_DEMO_FLAGON\n0xD5 _DEMO_GUESS\n";
    assert_eq!(names.read_labels("more.txt", more), Ok(1));
    assert_eq!(label(&names, 0xD1).as_deref(), Some("_GET_DEMO_FLAGON"));

    // A list refused labels nothing.
    let bad = "0xD6 _DEMO_GUESS\n0xD7 NOT_A_GUESS\n";
    assert!(names.read_labels("bad", bad).is_err());
    assert_eq!((names.labels.len(), names.label_lists.len()), (3, 2));

    // A natives list's name goes before any label.
    let natives = [(0xD2, named("SET_REAL_FLASK"))];
    names.natives = Some(NativeDb::from_names("natives", natives));
    assert_eq!(label(&names, 0xD2), None);
    let said = |file: &str, shown: usize| {
        format!(
            "`{file}` (labels (guesses): {shown} native(s) no natives list names, each written \
             after `_` as a guess at what it does, and no name)"
        )
    };
    assert_eq!(
        names.sources(),
        [said("labels.tsv", 0), said("more.txt", 2)]
    );

    let refused = |text: &str| match Names::default().read_labels("bad", text) {
        Err(NamesError::Line { line, what, .. }) => (line, what),
        other => panic!("`{text}` read as {other:?}"),
    };
    assert_eq!(
        refused("0xD1 GET_DEMO_FLASK\n"),
        (
            1,
            "`GET_DEMO_FLASK` is no label: one name that opens with `_`".to_owned()
        )
    );
    assert_eq!(refused("hash\tname\n0xD1\t_GET_DEMO_FLASK\n").0, 1);
    assert_eq!(refused("0xD1 _\n").0, 1);
    assert_eq!(
        refused("0xD1 _DEMO_A\n0xD1 _DEMO_B\n"),
        (
            2,
            "0x00000000000000D1 is labelled `_DEMO_B` here and `_DEMO_A` before".to_owned()
        )
    );
    assert_eq!(
        Names::default().read_labels("empty", "# no rows\n"),
        Err(NamesError::NoLabel {
            label: "empty".to_owned()
        })
    );
}

#[test]
fn every_row_of_the_tables_a_package_ships_is_read_by_its_column_names() {
    let (field, ty, method) = ("_wickTrim", "TallowPail", "GetWick");
    let (text, text64) = ("wick_trim", "tallow_pail");
    let tables = [
        format!(
            "field_hash64\tfield_hash32\tname\tevidence\n0x{:016X}\t0x{:08X}\t{field}\ta note\n",
            murmur_low(field.as_bytes()),
            folded32(field)
        ),
        format!(
            "type_hash64\ttype_hash32\tname\tevidence\n0x{:016X}\t0x{:08X}\t{ty}\ta note\n",
            murmur_low(ty.as_bytes()),
            folded32(ty)
        ),
        format!(
            "method_hash32\tname\tchance_of_accident\tevidence\trecords\n\
             0x{:08X}\t{method}\t0.0000\ta note\t12\n",
            kept32(method)
        ),
        format!(
            "hash32\ttext\tproof\toccurrences\tsources\tother_texts_same_hash\n\
             0x{:08X}\t{text}\ta note\t3\ta note\t\n",
            folded32(text)
        ),
        format!(
            "hash64\ttext\toccurrences\n0x{:016X}\t{text64}\t4\n",
            name_hash64(text64.as_bytes())
        ),
        "native_hash64\tcell\tkind\tnative\tdistinct_large_constants\tresolved\tgta5_parameter\n\
         0x00000000000000D1\t2\tHash\tDEMO.SET\t10\t9\tHash wick\n"
            .to_owned(),
        "hash\trva\tnamespace\tname\tnamed\targ_cells\treturn_cells\tfiles\tcalls\n\
         0x00000000000000AA\t0x01020304\t\t_0x00000000000000AA\t0\t1\t0\t1\t2\n"
            .to_owned(),
    ];
    let mut names = Names::default();
    for (n, table) in tables.iter().enumerate() {
        assert!(
            names.read_list(&format!("table {n}"), table).is_ok(),
            "{table}"
        );
    }
    assert_eq!(
        names.rows,
        [(1, 0), (1, 0), (1, 0), (1, 0), (1, 0), (0, 1), (0, 1)]
    );
    assert_eq!(names.field_name(murmur_low(field.as_bytes())), Some(field));
    assert_eq!(names.type_name(murmur_low(ty.as_bytes())), Some(ty));
    assert_eq!(names.method_name(kept32(method)), Some(method));
    assert_eq!(names.text(folded32(text)), Some(text));
    assert_eq!(names.text64(name_hash64(text64.as_bytes())), Some(text64));
    assert!(names.hash_slots.contains(&(0xD1, 2)));
    assert_eq!(names.native_at(0x0102_0304), Some(0xAA));
}
