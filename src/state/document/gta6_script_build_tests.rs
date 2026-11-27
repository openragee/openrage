use super::*;

#[test]
fn an_archive_is_told_by_its_first_bytes_and_nothing_else_is_one() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let archive = folder.path().join("demo.rpf");
    let mut bytes = RPF9_MAGIC.to_le_bytes().to_vec();
    bytes.extend([0u8; 12]);
    std::fs::write(&archive, &bytes).expect("written");
    assert!(is_archive(&archive));
    let program = folder.path().join("demo_program");
    std::fs::write(&program, [0u8; 64]).expect("written");
    assert!(!is_archive(&program));
    let short = folder.path().join("demo_short");
    std::fs::write(&short, &bytes[..2]).expect("written");
    assert!(!is_archive(&short));
    assert!(!is_archive(folder.path()));
    assert!(!is_archive(&folder.path().join("missing")));
}

#[test]
fn a_project_is_built_of_what_reads_and_a_stop_or_no_program_builds_none() {
    let (building, progress) = (Building::default(), Progress::default());
    let none = project_of(&[], &building, &progress);
    assert!(none.is_err_and(|e| e.contains("GTA VI program")));

    // A body that is no program gives the project nothing, and is still read twice.
    let bodies = vec![vec![0u8; 64]];
    let project = project_of(&bodies, &building, &progress).expect("a project of nothing");
    assert_eq!((building.read(), building.all()), (2, 2));
    assert!(!building.built());
    assert!(index::save(&project).is_ok());

    let stopping = Progress::default();
    stopping.stop();
    let stopped_at = project_of(&bodies, &Building::default(), &stopping);
    assert_eq!(stopped_at.err(), Some(stopped()));
}

#[test]
fn each_body_is_worked_in_its_order_and_one_that_faults_gives_nothing() {
    let bodies: Vec<Vec<u8>> = (0u8..9).map(|n| vec![n]).collect();
    let building = Building::default();
    let made = each(&bodies, &building, &Progress::default(), |bytes| {
        assert_ne!(bytes[0], 4, "a fault in one body");
        Some(bytes[0])
    });
    let expected: Vec<Option<u8>> = (0u8..9).map(|n| (n != 4).then_some(n)).collect();
    assert_eq!(made, expected);
    assert_eq!(building.read(), 9);
    assert!(each(&[], &building, &Progress::default(), |_| Some(1)).is_empty());
}

#[test]
fn a_project_takes_the_place_of_the_one_in_the_folder_and_no_part_of_it_is_left() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let folder = config.path().join("names");
    let first = index::save(&Project::default()).expect("a project");
    let written = put(&folder, &first).expect("the folder is made and the project written");
    assert_eq!(written, folder.join(PROJECT_FILE));
    std::fs::write(&written, "an older file").expect("written");
    put(&folder, &first).expect("written again");
    let read = std::fs::read(&written).expect("read back");
    assert!(index::load(&read).is_ok());
    assert!(!folder.join(PART_FILE).exists());
    // A folder the project cannot be written to is said, never a fault.
    let blocked = config.path().join("blocked");
    std::fs::write(&blocked, "a file where the folder would be").expect("written");
    assert!(put(&blocked, &first).is_err());
}
