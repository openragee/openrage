use super::*;

/// The widest line of `text` once it is indented, in characters.
fn widest(text: &str, indent: usize) -> usize {
    let first = 4 * indent;
    let lines = text.lines().enumerate();
    let width = |(n, line): (usize, &str)| line.chars().count() + if n == 0 { first } else { 0 };
    lines.map(width).max().unwrap_or(0)
}

/// `text` with every line end and the indentation after it made one space again.
fn joined(text: &str) -> String {
    let parts: Vec<&str> = text.lines().map(str::trim_start).collect();
    parts.join(" ")
}

#[test]
fn a_statement_that_fits_is_one_line() {
    let fits = "local_0 = shared.func_00A1B2(p0, p1) + 1;";
    assert_eq!(lay(fits, 3), fits);
    // The width counts the indentation: the same text one column too far in is cut.
    let long = format!("x = f({}, b);", "a".repeat(WIDTH - 15));
    assert_eq!(long.chars().count(), WIDTH - 4);
    assert_eq!(lay(&long, 1), long);
    assert_ne!(lay(&long, 2), long);
}

#[test]
fn a_list_fills_each_line_and_breaks_after_a_comma() {
    let args: Vec<String> = (0..40).map(|n| format!("local_{n}")).collect();
    let text = format!(
        "Cells3 v_000480_0 = shared.func_069F00({});",
        args.join(", ")
    );
    let laid = lay(&text, 1);
    assert!(laid.lines().count() > 1, "{laid}");
    assert!(widest(&laid, 1) <= WIDTH, "{laid}");
    // Each line but the last ends with the comma, and each line started stands one level in.
    let lines: Vec<&str> = laid.lines().collect();
    for line in &lines[..lines.len() - 1] {
        assert!(line.ends_with(','), "{laid}");
    }
    for line in &lines[1..] {
        assert!(line.starts_with("        local_"), "{laid}");
    }
    // A line is filled: no two neighbours would fit on one.
    for pair in lines.windows(2) {
        let together = pair[0].chars().count() + 1 + pair[1].trim_start().chars().count();
        assert!(together + 4 > WIDTH, "{laid}");
    }
    assert_eq!(joined(&laid), text, "only spaces became line ends");
}

#[test]
fn a_chain_of_tests_breaks_before_every_operator_or_before_none() {
    let test = "(uint)p0->f_18->f_10 == 0x9A3B5C7D";
    let text = format!("if ({test} || {test} || {test} || {test})");
    let laid = lay(&text, 1);
    let expected = format!("if ({test}\n        || {test}\n        || {test}\n        || {test})");
    assert_eq!(laid, expected);

    // The looser operator is the one cut: an `&&` inside an operand of `||` stays on its line.
    let both = format!("if ({test} && {test} || {test} && {test})");
    let laid = lay(&both, 0);
    assert_eq!(
        laid,
        format!("if ({test} && {test}\n    || {test} && {test})")
    );
}

#[test]
fn the_shallowest_brackets_are_cut_first() {
    let inner: Vec<String> = (0..8).map(|n| format!("local_{n}")).collect();
    let call = format!("func_000100({})", inner.join(", "));
    let text = format!("local_40 = func_000200({call}, {call});");
    assert!(text.chars().count() > WIDTH);
    let laid = lay(&text, 0);
    // The outer list is cut between its two arguments; neither inner list needs a cut.
    assert_eq!(
        laid,
        format!("local_40 = func_000200({call},\n    {call});")
    );
}

#[test]
fn a_string_and_a_comment_are_never_cut() {
    let said = "a, b || c + d ? e : f, ".repeat(8);
    let text = format!("DEMO.SAY(\"{said}\");");
    assert_eq!(lay(&text, 0), text, "nothing outside the string is a place");

    let escaped = format!("DEMO.SAY(\"\\\", {said}\", local_0, local_1);");
    let laid = lay(&escaped, 0);
    assert_eq!(joined(&laid), escaped);
    assert!(
        laid.starts_with(&format!("DEMO.SAY(\"\\\", {said}\",")),
        "{laid}"
    );

    let head = format!("void func_000040(Cell p0) // {said}");
    assert_eq!(lay(&head, 0), head, "what follows `//` is one piece");
}

#[test]
fn what_looks_like_a_place_and_is_none_stays_whole() {
    // A compound assignment, a null operator, a shift and the test of an equality are no places.
    let name = "local_0".repeat(14);
    for text in [
        format!("{name} |= {name};"),
        format!("{name} ?? {name};"),
        format!("{name} << {name};"),
        format!("{name}=={name};"),
    ] {
        assert!(text.chars().count() > WIDTH);
        assert_eq!(lay(&text, 0), text);
    }
}

#[test]
fn a_comparison_breaks_before_its_operator_where_nothing_looser_does() {
    let call = format!("DEMO.READ({})", "a".repeat(WIDTH - 20));
    for op in ["==", "!=", "<", "<=", ">", ">="] {
        let text = format!("if ({call} {op} 0x9A3B5C7D)");
        assert!(text.chars().count() > WIDTH);
        assert_eq!(lay(&text, 0), format!("if ({call}\n    {op} 0x9A3B5C7D)"));
    }
    // ⚠ The looser place is still the one cut: a comparison inside an operand of `&&` stays whole.
    let test = format!("{} == 4", "b".repeat(WIDTH / 2));
    let both = format!("if ({test} && {test})");
    assert_eq!(lay(&both, 0), format!("if ({test}\n    && {test})"));
}

#[test]
fn an_assignment_breaks_after_its_sign_only_where_the_statement_holds_no_other_place() {
    let call = format!("DEMO.READ({})", "a".repeat(WIDTH - 20));
    let text = format!("Cells2 local_0 = {call};");
    assert!(text.chars().count() > WIDTH);
    assert_eq!(lay(&text, 1), format!("Cells2 local_0 =\n        {call};"));
    // A member through a pointer and a sign are no places, and the `=` after them is one.
    let name = "local_0".repeat(14);
    let stored = format!("{name}->f_10 = -{name};");
    assert_eq!(lay(&stored, 0), format!("{name}->f_10 =\n    -{name};"));

    // A list is cut first, and the `=` of a field in braces is no place of the statement.
    let fields: Vec<String> = (0..12).map(|n| format!("f_{n:X}0 = local_{n}")).collect();
    let made = format!("Cell local_40 = new CThing {{ {} }};", fields.join(", "));
    assert!(made.chars().count() > WIDTH);
    let laid = lay(&made, 0);
    assert!(
        laid.starts_with("Cell local_40 = new CThing { f_00 = local_0,"),
        "{laid}"
    );
    assert_eq!(joined(&laid), made);
    // What fits is never cut.
    assert_eq!(lay("local_0 = local_1;", 0), "local_0 = local_1;");
}

#[test]
fn a_comment_is_lines_of_words_no_wider_than_the_width() {
    assert_eq!(comment("level: source", 0), "// level: source\n");
    assert_eq!(
        comment("values: 0, 1, 2", 2),
        "        // values: 0, 1, 2\n"
    );
    let words: Vec<String> = (0..60).map(|n| format!("word_{n}")).collect();
    let text = format!("names: {}", words.join(" "));
    let laid = comment(&text, 1);
    assert!(laid.ends_with("word_59\n"), "{laid}");
    let lines: Vec<&str> = laid.lines().collect();
    assert!(lines.len() > 1, "{laid}");
    assert!(
        lines[0].starts_with("    // names: word_0 word_1 "),
        "{laid}"
    );
    for line in &lines[1..] {
        assert!(line.starts_with("    //   word_"), "{laid}");
    }
    for line in &lines {
        assert!(
            line.chars().count() <= WIDTH && !line.ends_with(' '),
            "{laid}"
        );
    }
    // A line is filled: the first word of the next would not have fitted on it.
    for pair in lines.windows(2) {
        let next = pair[1].trim_start_matches("    //   ");
        let first = next.split(' ').next().unwrap_or_default();
        assert!(
            pair[0].chars().count() + 1 + first.chars().count() > WIDTH,
            "{laid}"
        );
    }
    // Only spaces became line ends.
    let said: Vec<&str> = lines
        .iter()
        .map(|line| line.trim_start().trim_start_matches('/').trim_start())
        .collect();
    assert_eq!(said.join(" "), text);
}

#[test]
fn a_statement_with_no_place_stays_long_and_deep_nesting_ends() {
    let long = "x".repeat(3 * WIDTH);
    assert_eq!(lay(&long, 2), long);

    // Brackets nested past the cuts one statement is given: the walk ends, and the text is whole.
    let mut nested = String::from("f(a, b)");
    for _ in 0..200 {
        nested = format!("f({nested}, {nested})");
        if nested.len() > 200_000 {
            break;
        }
    }
    let laid = lay(&nested, 0);
    assert_eq!(joined(&laid), nested);
}
