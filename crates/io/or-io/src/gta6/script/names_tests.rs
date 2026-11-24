use super::*;
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
