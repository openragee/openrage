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
    // A compound assignment, a null operator, a member through a pointer and a sign are no places.
    let name = "local_0".repeat(14);
    for text in [
        format!("{name} |= {name};"),
        format!("{name} ?? {name};"),
        format!("{name}->f_10 = -{name};"),
        format!("{name} = {name} << 2;"),
    ] {
        assert!(text.chars().count() > WIDTH);
        assert_eq!(lay(&text, 0), text);
    }
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
