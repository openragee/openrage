use super::*;

fn run(container: &str, name: &str, backend: &str, created: usize) -> Run {
    Run {
        container: container.to_owned(),
        adapter: Adapter {
            name: name.to_owned(),
            backend: backend.to_owned(),
            ..Default::default()
        },
        created,
        ..Default::default()
    }
}

fn report(runs: Vec<Run>) -> Report {
    Report {
        version: VERSION,
        containers: BTreeMap::new(),
        runs,
    }
}

#[test]
fn a_reason_loses_its_numbers_and_names() {
    assert_eq!(
        reason("Entry point 'main_12' at Vertex: location 31 of [44] is 0x1F"),
        "Entry point 'main_12' at Vertex: location N of [N] is N"
    );
    assert_eq!(
        reason(
            r"FXC D3DCompile error (0x80004005): C:\Users\Some One\Shader@0x0A(73,10-51): warning X3570: unrolled | C:\Users\Some One\Shader@0x0A(9,1-2): error X3004: Texture2D<float4> has no method"
        ),
        "FXC D3DCompile error (N): Shader@N(N,N-N): warning X3570: unrolled | Shader@N(N,N-N): error X3004: Texture2D<float4> has no method"
    );
    assert_eq!(
        reason("DXC compile error: hlsl.hlsl:9:4: error: no member | float4 _e94 = tex0_.Sample("),
        "DXC compile error: hlsl.hlsl:N:N: error: no member"
    );
    assert_eq!(
        reason(&"x".repeat(400)).chars().count(),
        241,
        "a long refusal is cut"
    );
}

#[test]
fn a_refusal_for_a_missing_feature_is_a_skip_by_its_name() {
    let mut run = Run::default();
    let lacking = "Shader validation error: Capability Capabilities(SUBGROUP) is not supported";
    run.count("3", &Outcome::Rejected(lacking.into()), 1.0);
    run.count(
        "4",
        &Outcome::Rejected("location 7 is not written".into()),
        1.0,
    );
    run.count("5", &Outcome::Created, 2.5);
    run.count("6", &Outcome::Skipped("SUBGROUP".into()), 0.0);
    run.count("7", &Outcome::Crashed("0xC0000005".into()), 0.0);
    run.count("8", &Outcome::TimedOut, 0.0);
    assert_eq!(run.skipped["SUBGROUP"], ["3", "6"]);
    assert_eq!(run.rejected["location N is not written"], ["4"]);
    assert_eq!((run.created, run.created_ms), (1, 2.5));
    assert_eq!((run.crashed_count(), run.timed_out.len()), (1, 1));
    assert_eq!(
        missing_feature("Features Features { features_wgpu: FeaturesWGPU(CLIP_DISTANCES), features_webgpu: FeaturesWebGPU(0x0) } are required but not enabled on the device"),
        Some("CLIP_DISTANCES".to_owned())
    );
    assert_eq!(missing_feature("the stages do not link"), None);
}

#[test]
fn a_merge_keeps_one_run_of_an_adapter_and_every_other() {
    let merged = merge(vec![
        report(vec![run("c", "A", "Vulkan", 1), run("c", "A", "Dx12", 2)]),
        report(vec![run("c", "A", "Vulkan", 9), run("c", "B", "Vulkan", 3)]),
    ])
    .unwrap();
    let created: Vec<usize> = merged.runs.iter().map(|r| r.created).collect();
    assert_eq!(created, [9, 2, 3], "the later run of one adapter wins");
    // A check run a range of units at a time adds up, and the same range read twice does not.
    let part = |from, to, unit: &str| {
        let mut part = run("c", "A", "Vulkan", 0);
        part.units = vec![(from, to)];
        part.count(unit, &Outcome::Created, 1.0);
        part.count(unit, &Outcome::Rejected("no".into()), 0.0);
        report(vec![part])
    };
    let whole = merge(vec![part(0, 4, "1"), part(4, 8, "5")]).unwrap();
    assert_eq!(whole.runs.len(), 1);
    assert_eq!(whole.runs[0].units, [(0, 4), (4, 8)]);
    assert_eq!(
        (whole.runs[0].created, &whole.runs[0].rejected["no"][..]),
        (2, &["1".to_owned(), "5".to_owned()][..])
    );
    let twice = merge(vec![part(4, 8, "5"), part(4, 8, "5")]).unwrap();
    assert_eq!(twice, part(4, 8, "5"));
    let old = Report {
        version: VERSION + 1,
        ..Default::default()
    };
    assert_eq!(merge(vec![old]), Err(MergeError::Version(VERSION + 1)));
}

#[test]
fn a_report_reads_back_and_its_table_names_each_run() {
    let mut first = run("abcdef", "A", "Vulkan", 4);
    first.count("v1+p2", &Outcome::Rejected("stage 3 mismatch".into()), 0.0);
    let written = report(vec![first, run("abcdef", "B", "Dx12", 5)]);
    let read: Report = serde_json::from_str(&serde_json::to_string(&written).unwrap()).unwrap();
    assert_eq!(read, written);
    let table = table(&read);
    assert!(table.contains("stage N mismatch"), "{table}");
    assert!(table.contains("A (Vulkan): v1+p2"), "{table}");
    assert!(table
        .lines()
        .any(|l| l.contains("B ") && l.contains("Dx12")));
}

#[test]
fn the_notes_of_two_reports_of_one_container_merge_without_repeats() {
    let noted = |units: &[&str]| {
        let mut report = report(Vec::new());
        let container = Container {
            notes: BTreeMap::from([(
                "fed nought".to_owned(),
                units.iter().map(|u| u.to_string()).collect(),
            )]),
            ..Default::default()
        };
        report.containers.insert("digest".to_owned(), container);
        report
    };
    let merged = merge(vec![noted(&["v1+p2", "v3+p4"]), noted(&["v3+p4", "v5+p6"])]).unwrap();
    assert_eq!(
        merged.containers["digest"].notes["fed nought"],
        ["v1+p2", "v3+p4", "v5+p6"]
    );
    assert!(table(&merged).contains("3  noted: fed nought"));
}
