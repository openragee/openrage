use super::*;

fn member(name: &str, value: impl std::fmt::Debug) -> String {
    format!("\n      <{name} value=\"{value:?}\" />")
}

fn corners([min_x, max_x, min_y, max_y]: [i16; 4]) -> String {
    [
        member("minX", min_x),
        member("maxX", max_x),
        member("minY", min_y),
        member("maxY", max_y),
    ]
    .concat()
}

fn list(name: &str, items: &[String]) -> String {
    let items: String = (items.iter())
        .map(|item| format!("\n    <Item>{item}\n    </Item>"))
        .collect();
    format!("\n  <{name}>{items}\n  </{name}>")
}

// A float is written here as `0.0` and by the writer as `0`, so a save that gives this text back
// kept what the document stated.
fn water_item(quad: &CWaterQuad) -> String {
    [
        corners(quad.rectangle_stated()),
        member("Type", quad.r#type),
        member("IsInvisible", quad.is_invisible),
        member("HasLimitedDepth", quad.has_limited_depth),
        member("z", quad.z),
        member("a1", quad.a1),
        member("a2", quad.a2),
        member("a3", quad.a3),
        member("a4", quad.a4),
        member("NoStencil", quad.no_stencil),
    ]
    .concat()
}

fn calming_item(quad: &CCalmingQuad) -> String {
    let dampening = quad.dampening.map(|value| member("fDampening", value));
    [
        corners([quad.min_x, quad.max_x, quad.min_y, quad.max_y]),
        dampening.unwrap_or_default(),
    ]
    .concat()
}

fn wave_item(quad: &CWaveQuad) -> String {
    let stated = |name: &str, value: Option<f32>| value.map(|value| member(name, value));
    [
        corners([quad.min_x, quad.max_x, quad.min_y, quad.max_y]),
        stated("Amplitude", quad.amplitude).unwrap_or_default(),
        stated("XDirection", quad.x_direction).unwrap_or_default(),
        stated("YDirection", quad.y_direction).unwrap_or_default(),
    ]
    .concat()
}

fn document(water: &[CWaterQuad], calming: &[CCalmingQuad], wave: &[CWaveQuad]) -> String {
    let items = |items: Vec<String>, name: &str| match items.is_empty() {
        true => String::new(),
        false => list(name, &items),
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<WaterData>{}{}{}\n</WaterData>\n",
        items(water.iter().map(water_item).collect(), "WaterQuads"),
        items(calming.iter().map(calming_item).collect(), "CalmingQuads"),
        items(wave.iter().map(wave_item).collect(), "WaveQuads"),
    )
}

fn water(corners: [i16; 4], z: f32, kind: u8) -> CWaterQuad {
    let [min_x, max_x, min_y, max_y] = corners;
    CWaterQuad {
        min_x,
        max_x,
        min_y,
        max_y,
        z,
        r#type: kind,
        ..CWaterQuad::default()
    }
}

fn sample() -> (Vec<CWaterQuad>, Vec<CCalmingQuad>, Vec<CWaveQuad>) {
    let pool = CWaterQuad {
        is_invisible: true,
        has_limited_depth: true,
        no_stencil: true,
        a1: 0,
        a2: 255,
        a3: 7,
        a4: 200,
        ..water([-40, -8, 12, 60], 31.25, 3)
    };
    let calming = CCalmingQuad {
        min_x: 100,
        max_x: 140,
        min_y: -20,
        max_y: 20,
        dampening: Some(0.0),
        ..CCalmingQuad::default()
    };
    let wave = CWaveQuad {
        min_x: -300,
        max_x: -100,
        min_y: 400,
        max_y: 500,
        amplitude: Some(0.25),
        x_direction: Some(-0.6),
        y_direction: Some(0.8),
        ..CWaveQuad::default()
    };
    let still = CWaveQuad {
        amplitude: None,
        ..wave.clone()
    };
    (
        vec![water([-200, 200, -100, 100], 0.0, 0), pool],
        vec![calming],
        vec![wave, still],
    )
}

fn open(text: &str) -> CWaterData {
    CWaterData::open(text.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn saved(data: &CWaterData) -> String {
    String::from_utf8(data.save().expect("saves")).expect("a save is text")
}

fn refusal(text: &str) -> String {
    match CWaterData::open(text.as_bytes()) {
        Ok(_) => panic!("opened:\n{text}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_document_reads_to_its_quads_and_saves_to_its_fixed_point() {
    let (water, calming, wave) = sample();
    let text = document(&water, &calming, &wave);
    let data = open(&text);
    assert_eq!(data.water_quads, water);
    assert_eq!(data.calming_quads, calming);
    assert_eq!(data.wave_quads, wave);
    assert!(data.unnamed().is_empty(), "{:?}", data.unnamed());

    let first = saved(&data);
    assert_eq!(first, text);
    let again = open(&first);
    assert_eq!(again, data);
    assert_eq!(saved(&again), first);
}

#[test]
fn a_member_an_item_leaves_out_reads_as_the_games_default_and_is_not_added_by_a_save() {
    let text = "<WaterData><WaterQuads><Item>\
        <minX value=\"1\"/><maxX value=\"2\"/><minY value=\"3\"/><maxY value=\"4\"/>\
        </Item></WaterQuads><CalmingQuads/></WaterData>";
    let data = open(text);
    assert_eq!(data.water_quads, [water([1, 2, 3, 4], 0.0, 0)]);
    let quad = &data.water_quads[0];
    assert_eq!(
        [quad.a1, quad.a2, quad.a3, quad.a4],
        [26; 4],
        "the opacity an item does not state"
    );
    assert_eq!(saved(&data), text);
}

#[test]
fn an_element_and_an_attribute_the_reader_does_not_name_survive_a_read_and_a_save() {
    let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- written by hand -->
<WaterData version="2">
  <FogQuads>
    <Item><minX value="1" /><c0 value="0x11223344" /></Item>
  </FogQuads>
  <WaterQuads count="1">
    <Shore side="south" />
    <!-- the lake -->
    <Item kind="lake">
      <minX value="-10" unit="m" />
      <maxX value="10" />
      <minY value="-5" />
      <maxY value="5" />
      <z value="2.50" />
      <oceanQuad value="true" />
      <z value="9" />
      <Type value="0">stray</Type>
    </Item>
    <Border side="north" />
  </WaterQuads>
  <WaterQuads><Item><minX value="0" /></Item></WaterQuads>
</WaterData>
"#;
    let mut data = open(text);
    assert_eq!(data.water_quads, [water([-10, 10, -5, 5], 2.5, 0)]);
    let unnamed: Vec<String> = data.unnamed().into_iter().collect();
    assert_eq!(
        unnamed,
        [
            "WaterData/FogQuads",
            "WaterData/WaterQuads",
            "WaterData/WaterQuads/Border",
            "WaterData/WaterQuads/Item/Type/#text",
            "WaterData/WaterQuads/Item/minX@unit",
            "WaterData/WaterQuads/Item/oceanQuad",
            "WaterData/WaterQuads/Item/z",
            "WaterData/WaterQuads/Item@kind",
            "WaterData/WaterQuads/Shore",
            "WaterData/WaterQuads@count",
            "WaterData@version",
        ]
    );
    assert_eq!(saved(&data), text);

    // A changed value is written beside them, and they stay.
    data.water_quads[0].min_x = -12;
    data.water_quads[0].z = 3.0;
    let changed = text
        .replace("<minX value=\"-10\" unit", "<minX value=\"-12\" unit")
        .replace("\"2.50\"", "\"3\"");
    assert_ne!(changed, text);
    assert_eq!(saved(&data), changed);
    assert_eq!(open(&changed).unnamed(), data.unnamed());
}

#[test]
fn a_text_is_kept_however_it_is_laid_out() {
    let text = "\u{feff}<?xml version='1.0'?>\r\n<?tool keep?>\r\n<WaterData\r\n  >\r\n\
        <WaterQuads >\t<Item\n><minX  value = '4'/><maxX value=\"6\"\n/><minY value='-1' />\
        <maxY value='1'></maxY ><![CDATA[<z value=\"7\"/>]]><?z 7?><!-- <z value=\"7\"/> -->\
        </Item ></WaterQuads >\r\n<CalmingQuads/><WaveQuads></WaveQuads>\
        </WaterData >\r\n<!-- end -->\r\n";
    let data = open(text);
    assert_eq!(data.water_quads, [water([4, 6, -1, 1], 0.0, 0)]);
    assert_eq!(saved(&data), text);
}

#[test]
fn the_height_at_a_point_inside_a_quad_on_its_edge_and_outside_every_quad() {
    let (first, second) = (
        water([0, 10, 0, 20], 3.5, 0),
        water([5, 30, 10, 40], 8.0, 0),
    );
    let data = open(&document(&[first, second], &[], &[]));
    assert_eq!(data.height_at(2.0, 3.0), Some(3.5), "inside");
    for (x, y) in [(0.0, 7.0), (10.0, 7.0), (4.0, 0.0), (4.0, 20.0), (0.0, 0.0)] {
        assert_eq!(data.height_at(x, y), Some(3.5), "on an edge at ({x}, {y})");
    }
    assert_eq!(data.height_at(30.0, 40.0), Some(8.0), "the far corner");
    assert_eq!(data.height_at(7.0, 15.0), Some(3.5), "the first of two");
    assert_eq!(data.height_at(20.0, 15.0), Some(8.0), "the second alone");
    for (x, y) in [
        (-0.5, 5.0),
        (10.5, 5.0),
        (3.0, -0.5),
        (3.0, 40.5),
        (31.0, 41.0),
    ] {
        assert_eq!(data.height_at(x, y), None, "outside at ({x}, {y})");
    }
    assert_eq!(CWaterData::default().height_at(0.0, 0.0), None);
}

#[test]
fn a_cut_quad_holds_water_on_the_half_that_keeps_its_corner() {
    // A point near each corner of a rectangle 40 wide and 20 deep: south-west, north-west,
    // north-east and south-east, the corner types 1 to 4 keep.
    let near = [(1.0, 0.5), (1.0, 19.5), (39.0, 19.5), (39.0, 0.5)];
    for kind in 1..=4u8 {
        let quad = water([0, 40, 0, 20], 1.0, kind);
        let kept = usize::from(kind - 1);
        for (corner, &(x, y)) in near.iter().enumerate() {
            // The corner opposite the kept one is cut away; the two beside it lie on the cut.
            let wet = corner != (kept + 2) % 4;
            assert_eq!(quad.covers(x, y), wet, "type {kind} at ({x}, {y})");
        }
        assert!(
            quad.covers(20.0, 10.0),
            "type {kind}: the middle is on the cut"
        );
        assert!(
            !quad.covers(41.0, 10.0),
            "type {kind}: outside the rectangle"
        );
    }
    // The cut passes through the middle, at (20, 10). The halves under it hold a point that far
    // past it and no further; the halves over it hold none short of it.
    let (within, beyond) = (10.0 + CUT_SLACK / 2.0, 10.0 + CUT_SLACK * 2.0);
    for kind in [1, 4] {
        let quad = water([0, 40, 0, 20], 1.0, kind);
        assert!(quad.covers(20.0, within), "type {kind}: within the slack");
        assert!(!quad.covers(20.0, beyond), "type {kind}: past the slack");
    }
    for kind in [2, 3] {
        let quad = water([0, 40, 0, 20], 1.0, kind);
        assert!(!quad.covers(20.0, 10.0 - CUT_SLACK / 2.0), "type {kind}");
    }
    // A type past 5 is cut as 3 when odd and as 4 when even.
    for (kind, cut_as) in [(6, 4), (7, 3), (8, 4), (255, 3)] {
        let (quad, like) = (
            water([0, 40, 0, 20], 1.0, kind),
            water([0, 40, 0, 20], 1.0, cut_as),
        );
        for &(x, y) in &near {
            assert_eq!(quad.covers(x, y), like.covers(x, y), "type {kind}");
        }
    }
    // A quad of no width has no diagonal to be cut by, and holds water along its line.
    for kind in 0..=4 {
        let line = water([5, 5, 0, 20], 1.0, kind);
        assert!(line.covers(5.0, 10.0), "type {kind}");
        assert!(!line.covers(5.5, 10.0), "type {kind}");
    }
    let deleted = water([0, 40, 0, 20], 1.0, DELETED);
    assert!(!deleted.covers(20.0, 10.0));
    let whole = water([0, 40, 0, 20], 1.0, 0);
    assert!(near.iter().all(|&(x, y)| whole.covers(x, y)));

    let cut = water([0, 40, 0, 20], 6.0, 1);
    let data = open(&document(&[cut.clone(), whole], &[], &[]));
    assert_eq!(data.height_at(1.0, 0.5), Some(6.0), "on the kept half");
    assert_eq!(
        data.height_at(39.0, 19.5),
        Some(1.0),
        "past the cut, the quad under it"
    );
    let data = open(&document(&[cut, deleted], &[], &[]));
    assert_eq!(
        data.height_at(39.0, 19.5),
        None,
        "past the cut, a quad left out"
    );
}

#[test]
fn a_quad_given_with_its_corners_in_the_other_order_reads_to_the_same_rectangle() {
    let straight = open(&document(&[water([-30, 50, 10, 90], 4.0, 0)], &[], &[]));
    let turned = open(&document(&[water([50, -30, 90, 10], 4.0, 0)], &[], &[]));
    let expected = Rectangle {
        min: [-30.0, 10.0],
        max: [50.0, 90.0],
    };
    assert_eq!(straight.water_quads[0].rectangle(), expected);
    assert_eq!(turned.water_quads[0].rectangle(), expected);
    assert_eq!(turned.water_quads[0].rectangle_stated(), [50, -30, 90, 10]);
    assert_eq!(turned.height_at(0.0, 50.0), Some(4.0));
    assert_eq!(turned.height_at(-31.0, 50.0), None);

    let calming = CCalmingQuad {
        min_x: 8,
        max_x: 2,
        min_y: 1,
        max_y: -1,
        ..CCalmingQuad::default()
    };
    let wave = CWaveQuad {
        min_x: 8,
        max_x: 2,
        min_y: 1,
        max_y: -1,
        ..CWaveQuad::default()
    };
    let expected = Rectangle {
        min: [2.0, -1.0],
        max: [8.0, 1.0],
    };
    let data = open(&document(&[], &[calming], &[wave]));
    assert_eq!(data.calming_quads[0].rectangle(), expected);
    assert_eq!(data.wave_quads[0].rectangle(), expected);
}

#[test]
fn each_refusal_names_its_element() {
    assert_eq!(
        refusal("<?xml version=\"1.0\"?>\n<WaterTune><WaterQuads/></WaterTune>"),
        "the document's root is <WaterTune>, not <WaterData>"
    );

    let (water, calming, wave) = sample();
    let text = document(&water, &calming, &wave);
    for (list, item, corner, line) in [
        ("WaterQuads", 0, "minX", member("minX", -200)),
        ("WaterQuads", 1, "maxY", member("maxY", 60)),
        ("CalmingQuads", 0, "maxX", member("maxX", 140)),
        ("WaveQuads", 1, "minY", member("minY", 400)),
    ] {
        // The second wave quad repeats the first, so its line is the last of its kind.
        let at = text.rfind(&line).expect("the document states the corner");
        let without = [&text[..at], &text[at + line.len()..]].concat();
        assert_eq!(
            refusal(&without),
            format!("item {item} of <{list}>, counted from 0, states no <{corner}>")
        );
    }

    for (element, from, to, wanted) in [
        (
            "minX",
            "\"-200\"",
            "\"-200.5\"",
            "a whole number from -32768 to 32767",
        ),
        (
            "maxX",
            "\"200\"",
            "\"32768\"",
            "a whole number from -32768 to 32767",
        ),
        ("Type", "\"3\"", "\"256\"", "a whole number from 0 to 255"),
        ("a2", "\"255\"", "\"-1\"", "a whole number from 0 to 255"),
        ("IsInvisible", "\"true\"", "\"yes\"", "true or false"),
        ("z", "\"31.25\"", "\"high\"", "a finite number"),
        ("z", "\"31.25\"", "\"inf\"", "a finite number"),
        ("z", "\"31.25\"", "\"NaN\"", "a finite number"),
        ("z", "\"31.25\"", "\"\"", "a finite number"),
    ] {
        let stated = format!("<{element} value={from} />");
        assert_eq!(text.matches(&stated).count(), 1, "{stated}");
        let broken = text.replace(&stated, &format!("<{element} value={to} />"));
        let item = usize::from(element != "minX" && element != "maxX");
        let text = to.trim_matches('"');
        assert_eq!(
            refusal(&broken),
            format!(
                "<{element}> of item {item} of <WaterQuads>, counted from 0, holds `{text}`, which is not {wanted}"
            )
        );
    }
    assert_eq!(
        refusal(&text.replace("<fDampening value=\"0.0\" />", "<fDampening value=\"1,5\" />")),
        "<fDampening> of item 0 of <CalmingQuads>, counted from 0, holds `1,5`, which is not a finite number"
    );
    assert_eq!(
        refusal(&text.replace("<XDirection value=\"-0.6\" />", "<XDirection value=\"west\" />")),
        "<XDirection> of item 0 of <WaveQuads>, counted from 0, holds `west`, which is not a finite number"
    );
    assert_eq!(
        refusal(&text.replace(
            "<YDirection value=\"0.8\" />",
            "<YDirection>0.8</YDirection>"
        )),
        "<YDirection> of item 0 of <WaveQuads>, counted from 0, has no `value`"
    );
}

#[test]
fn a_text_that_is_not_a_document_is_refused_where_it_breaks() {
    for (text, why) in [
        ("", "line 1: no root element"),
        ("water", "line 1: no root element"),
        (
            "<?xml version=\"1.0\"?>\n<!DOCTYPE WaterData>\n<WaterData/>",
            "line 2: a document type declaration, which is not read",
        ),
        (
            "<WaterData>\n<WaterQuads>\n</WaterData>",
            "line 3: <WaterQuads> is closed by another tag",
        ),
        (
            "<WaterData>\n<WaterQuads>",
            "line 2: <WaterQuads> is never closed",
        ),
        (
            "<WaterData a=1/>",
            "line 1: attribute a of <WaterData> is not quoted",
        ),
        (
            "<WaterData a='1/>",
            "line 1: attribute a of <WaterData> does not end",
        ),
        (
            "<WaterData a/>",
            "line 1: attribute a of <WaterData> has no value",
        ),
        (
            "<WaterData/>\n<WaterData/>",
            "line 2: text after the root element",
        ),
        (
            "<WaterData><!-- </WaterData>",
            "line 1: a comment that does not end",
        ),
        (
            "<WaterData></WaterDatas>",
            "line 1: <WaterData> is closed by another tag",
        ),
        (
            "<WaterData>\n<Item kind='a' kind='b'/></WaterData>",
            "line 2: attribute kind of <Item> is stated twice",
        ),
        (
            "<WaterData>\n\n<!ELEMENT WaterQuads ANY></WaterData>",
            "line 3: a declaration, which is not read",
        ),
        ("<WaterData><></WaterData>", "line 1: a tag with no name"),
        (
            "<WaterData a='1'b='2'/>",
            "line 1: the tag of <WaterData> does not end",
        ),
    ] {
        assert_eq!(
            refusal(text),
            format!("the water file is not XML this reads, at {why}"),
            "{text}"
        );
    }
    let deep = format!("<WaterData>{}", "<a>".repeat(100));
    assert!(
        refusal(&deep).ends_with("elements nested deeper than 64"),
        "{}",
        refusal(&deep)
    );
    assert_eq!(
        CWaterData::open(b"<WaterData>\xff</WaterData>").unwrap_err(),
        WaterError::NotText(11)
    );
}

#[test]
fn a_changed_value_is_written_and_what_was_not_changed_keeps_its_text() {
    let (water, calming, wave) = sample();
    let text = document(&water, &calming, &wave);
    let mut data = open(&text);
    data.water_quads[0].max_y = 150;
    data.water_quads[1].z = 32.0;
    data.water_quads[1].is_invisible = false;
    data.calming_quads[0].dampening = None;
    data.wave_quads[0].x_direction = Some(1.0);
    data.wave_quads[1].amplitude = Some(0.5);

    let changed = text
        .replacen("<maxY value=\"100\" />", "<maxY value=\"150\" />", 1)
        .replace("<z value=\"31.25\" />", "<z value=\"32\" />")
        .replace(
            "<IsInvisible value=\"true\" />",
            "<IsInvisible value=\"false\" />",
        )
        .replace("\n      <fDampening value=\"0.0\" />", "")
        .replacen(
            "<XDirection value=\"-0.6\" />",
            "<XDirection value=\"1\" />",
            1,
        );
    // The second wave quad states no amplitude, and takes one after its last element.
    let tail = "<YDirection value=\"0.8\" />\n    </Item>\n  </WaveQuads>";
    let changed = changed.replace(
        tail,
        &tail.replacen("/>", "/>\n      <Amplitude value=\"0.5\" />", 1),
    );
    assert_eq!(text.matches(tail).count(), 1);
    let written = saved(&data);
    assert_eq!(written, changed);
    // The first quad's `z` is `0.0` in the text and was not touched.
    assert!(written.contains("<z value=\"0.0\" />"));

    let again = open(&written);
    assert_eq!(again, data);
    assert_eq!(saved(&again), written);
}

#[test]
fn a_value_the_file_cannot_state_is_refused_at_the_save() {
    let (water, calming, wave) = sample();
    let mut data = open(&document(&water, &calming, &wave));
    data.water_quads[1].z = f32::NAN;
    assert_eq!(
        data.save().unwrap_err().to_string(),
        "<z> of item 1 of <WaterQuads>, counted from 0, is NaN, which the file cannot state"
    );
    data.water_quads[1].z = 1.0;
    data.wave_quads[1].y_direction = Some(f32::INFINITY);
    assert_eq!(
        data.save().unwrap_err().to_string(),
        "<YDirection> of item 1 of <WaveQuads>, counted from 0, is inf, which the file cannot state"
    );
}

#[test]
fn quads_added_and_removed_save_and_read_back() {
    let (water, calming, wave) = sample();
    // No wave list, and a calming list closed in its own tag.
    let text =
        document(&water, &[], &[]).replace("\n</WaterData>", "\n  <CalmingQuads />\n</WaterData>");
    let mut data = open(&text);
    let first = data.water_quads.remove(0);
    data.water_quads.push(first);
    data.water_quads.push(self::water([1, 2, 3, 4], 1.5, 4));
    data.calming_quads = calming.clone();
    data.wave_quads = wave.clone();

    let written = saved(&data);
    let again = open(&written);
    assert_eq!(again.water_quads.len(), 3);
    assert_eq!(again, data);
    assert!(again.unnamed().is_empty(), "{:?}", again.unnamed());
    assert_eq!(saved(&again), written);
    // The quad the document held keeps its text, behind the one that followed it.
    assert!(written.contains(&format!(
        "<WaterQuads>\n    <Item>{}\n    </Item>\n    <Item>{}\n    </Item>",
        water_item(&water[1]),
        water_item(&water[0]),
    )));
    assert!(
        written.contains("\n  <CalmingQuads>\n    <Item>"),
        "{written}"
    );
    assert!(
        written.contains("\n  </CalmingQuads>\n  <WaveQuads>\n    <Item>"),
        "{written}"
    );
    assert!(
        written.ends_with("\n  </WaveQuads>\n</WaterData>\n"),
        "{written}"
    );
}

#[test]
fn a_file_authored_from_nothing_is_the_document_of_its_quads() {
    let empty = CWaterData::default();
    assert_eq!(
        saved(&empty),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<WaterData></WaterData>\n"
    );
    assert_eq!(open(&saved(&empty)), empty);

    // Halves and quarters are written alike with and without a fraction's zero.
    let pool = CWaterQuad {
        has_limited_depth: true,
        a1: 40,
        ..water([-4, 4, -2, 2], 12.5, 2)
    };
    let calming = CCalmingQuad {
        min_x: 3,
        max_x: 9,
        min_y: -9,
        max_y: -3,
        dampening: Some(0.25),
        ..CCalmingQuad::default()
    };
    let wave = CWaveQuad {
        min_x: 3,
        max_x: 9,
        min_y: -9,
        max_y: -3,
        amplitude: Some(1.5),
        x_direction: None,
        y_direction: Some(-0.5),
        ..CWaveQuad::default()
    };
    let authored = CWaterData {
        water_quads: vec![pool.clone()],
        calming_quads: vec![calming.clone()],
        wave_quads: vec![wave.clone()],
        ..CWaterData::default()
    };
    let written = saved(&authored);
    assert_eq!(written, document(&[pool], &[calming], &[wave]));
    assert_eq!(open(&written), authored);
    assert_eq!(saved(&open(&written)), written);
}

#[test]
fn a_water_file_is_known_by_its_name() {
    for name in [
        "water.xml",
        "WATER.XML",
        "water_heistisland.xml",
        "Water_Island.XML",
    ] {
        assert!(is_water_file(name), "{name}");
    }
    for name in [
        "watertune.xml",
        "water.xml.bak",
        "water_island.xml.bak",
        "water.meta",
        "underwater.xml",
        "water_.dat",
    ] {
        assert!(!is_water_file(name), "{name}");
    }
}

#[test]
fn a_zero_keeps_the_sign_it_is_given() {
    let stated = member("z", 0.0_f32);
    let text = document(&[water([0, 8, 0, 4], 0.0, 0)], &[], &[]);
    assert_eq!(text.matches(&stated).count(), 1);
    let bare = text.replace(&stated, "");
    let below = text.replace(&stated, "\n      <z value=\"-0.0\" />");

    for (from, to) in [
        (&text, "\n      <z value=\"-0\" />"),
        (&below, "\n      <z value=\"-0.0\" />"),
    ] {
        let mut data = open(from);
        data.water_quads[0].z = -0.0;
        assert_eq!(saved(&data), text.replace(&stated, to));
    }
    // An item that states no height reads as zero above, so one below is stated.
    let mut data = open(&bare);
    data.water_quads[0].z = -0.0;
    let written = saved(&data);
    assert_ne!(written, bare);
    assert!(open(&written).water_quads[0].z.is_sign_negative());
}

#[test]
fn a_member_stated_twice_is_left_out_whole_when_its_value_is_taken_away() {
    let wave = CWaveQuad {
        max_x: 8,
        max_y: 4,
        amplitude: Some(1.5),
        ..CWaveQuad::default()
    };
    let once = document(&[], &[], std::slice::from_ref(&wave));
    let first = member("Amplitude", 1.5_f32);
    let twice = once.replace(
        &first,
        &[&first, "<!-- was -->", &member("Amplitude", 2.5_f32)].concat(),
    );
    assert_ne!(twice, once);

    let mut data = open(&twice);
    assert_eq!(data.wave_quads, [wave]);
    assert_eq!(saved(&data), twice);
    data.wave_quads[0].amplitude = None;
    let written = saved(&data);
    assert_eq!(written, once.replace(&first, "<!-- was -->"));
    let again = open(&written);
    assert_eq!(again.wave_quads[0].amplitude, None);
    assert_eq!(again, data);
}

#[test]
fn what_the_document_did_not_hold_is_laid_out_as_the_document_is() {
    // Tabs, and lines ended by a carriage return and a line feed.
    let corners = "\r\n\t\t\t<minX value=\"1\"/>\r\n\t\t\t<maxX value=\"2\"/>\r\n\t\t\t<minY value=\"3\"/>\r\n\t\t\t<maxY value=\"4\"/>";
    let text = format!(
        "<WaterData>\r\n\t<WaterQuads>\r\n\t\t<Item>{corners}\r\n\t\t</Item>\r\n\t</WaterQuads>\r\n</WaterData>\r\n"
    );
    let mut data = open(&text);
    data.water_quads[0].z = 2.5;
    data.water_quads.push(CWaterQuad {
        min_x: 1,
        max_x: 2,
        min_y: 3,
        max_y: 4,
        z: 2.5,
        ..CWaterQuad::default()
    });
    data.calming_quads.push(CCalmingQuad {
        min_x: 1,
        max_x: 2,
        min_y: 3,
        max_y: 4,
        ..CCalmingQuad::default()
    });
    let lead = "\r\n\t\t\t";
    let spaced = |members: &[&str]| -> String {
        (members.iter())
            .map(|member| format!("{lead}<{member} />"))
            .collect()
    };
    let item = [
        spaced(&[
            "minX value=\"1\"",
            "maxX value=\"2\"",
            "minY value=\"3\"",
            "maxY value=\"4\"",
            "Type value=\"0\"",
            "IsInvisible value=\"false\"",
            "HasLimitedDepth value=\"false\"",
            "z value=\"2.5\"",
            "a1 value=\"26\"",
            "a2 value=\"26\"",
            "a3 value=\"26\"",
            "a4 value=\"26\"",
            "NoStencil value=\"false\"",
        ]),
        "\r\n\t\t".to_string(),
    ]
    .concat();
    let calming = [
        spaced(&[
            "minX value=\"1\"",
            "maxX value=\"2\"",
            "minY value=\"3\"",
            "maxY value=\"4\"",
        ]),
        "\r\n\t\t".to_string(),
    ]
    .concat();
    let expected = format!(
        "<WaterData>\r\n\t<WaterQuads>\r\n\t\t<Item>{corners}{lead}<z value=\"2.5\" />\r\n\t\t</Item>\
         \r\n\t\t<Item>{item}</Item>\r\n\t</WaterQuads>\
         \r\n\t<CalmingQuads>\r\n\t\t<Item>{calming}</Item>\r\n\t</CalmingQuads>\r\n</WaterData>\r\n"
    );
    let written = saved(&data);
    assert_eq!(written, expected);
    assert_eq!(saved(&open(&written)), written);

    // A document on one line states no layout: what is added is two spaces a level, and its
    // lines end as the document's do.
    let mut data = open("<WaterData />\r\n");
    data.calming_quads.push(CCalmingQuad::default());
    let zeroes: String = ["minX", "maxX", "minY", "maxY"]
        .map(|corner| format!("\r\n      <{corner} value=\"0\" />"))
        .concat();
    assert_eq!(
        saved(&data),
        format!(
            "<WaterData>\r\n  <CalmingQuads>\r\n    <Item>{zeroes}\r\n    </Item>\r\n  </CalmingQuads>\r\n</WaterData>\r\n"
        )
    );
    let mut data = open("<WaterData/>");
    data.calming_quads.push(CCalmingQuad::default());
    assert_eq!(
        saved(&data),
        format!(
            "<WaterData>\n  <CalmingQuads>\n    <Item>{}\n    </Item>\n  </CalmingQuads>\n</WaterData>",
            zeroes.replace('\r', "")
        )
    );

    // A list that holds no item states how deep it stands, and an item and a member stand deeper
    // by as much; a list closed right behind its opening tag closes on a line of its own.
    let tabbed = |inside: &str| {
        format!("<WaterData>\n\t<CalmingQuads>{inside}\n\t</CalmingQuads>\n</WaterData>\n")
    };
    let mut data = open(&tabbed(""));
    data.calming_quads.push(CCalmingQuad::default());
    let item = zeroes.replace("\r\n      ", "\n\t\t\t");
    assert_eq!(
        saved(&data),
        tabbed(&format!("\n\t\t<Item>{item}\n\t\t</Item>"))
    );
    let mut data = open("<WaterData><CalmingQuads></CalmingQuads></WaterData>");
    data.calming_quads.push(CCalmingQuad::default());
    assert_eq!(
        saved(&data),
        format!(
            "<WaterData><CalmingQuads>\n    <Item>{}\n    </Item>\n  </CalmingQuads></WaterData>",
            zeroes.replace('\r', "")
        )
    );

    let apart = |lead: &str, members: &str| {
        ["minX", "maxX", "minY", "maxY"]
            .map(|corner| format!("{members}<{corner} value=\"0\" />"))
            .concat()
            + lead
    };
    // An item is led by the white space right before it, and what closes a list stays behind
    // an item added last.
    let item = apart("\n\t", "\n\t\t");
    let held = |items: &str| {
        format!("<WaterData><CalmingQuads>\n<!-- first -->{items}<!-- last --></CalmingQuads></WaterData>")
    };
    let mut data = open(&held(&format!("\n\t<Item>{item}</Item>")));
    data.calming_quads.push(CCalmingQuad::default());
    assert_eq!(
        saved(&data),
        held(&format!("\n\t<Item>{item}</Item>\n\t<Item>{item}</Item>"))
    );

    // Where the lists are laid out differently, the water quads' items say how.
    let held = format!(
        "<WaterData><WaterQuads> <Item>{}</Item></WaterQuads><CalmingQuads>\n\t<Item>{}</Item></CalmingQuads>",
        apart(" ", " "),
        apart("\n\t", "\n\t\t"),
    );
    let mut data = open(&format!("{held}</WaterData>"));
    data.wave_quads.push(CWaveQuad::default());
    assert_eq!(
        saved(&data),
        format!(
            "{held}\n  <WaveQuads> <Item>{}</Item>\n  </WaveQuads>\n</WaterData>",
            apart(" ", " ")
        )
    );
}
