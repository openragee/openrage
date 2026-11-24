use super::*;

fn archetype(word: u64, model: u64, dictionary: u64, name: u32) -> ArchetypeRow {
    ArchetypeRow {
        word,
        model,
        dictionary,
        name,
        lod_dist: 100.0,
        centre: [0.0, 0.0, 1.0],
        radius: 2.0,
        flags: 0,
        pad: 0,
    }
}

fn body(word: u64, archive: u32, offset: u64) -> BodyRow {
    BodyRow {
        word,
        offset,
        archive,
        pad: 0,
    }
}

fn map(name: u32, held: u32) -> MapRow {
    MapRow {
        archive: 0,
        name,
        block: 7,
        flags: 0,
        content_flags: 1,
        held,
        parents_from: 0,
        parents_len: 0,
        offset: u64::from(name) * 16,
        boxes: [
            -10.0, -10.0, -10.0, 10.0, 10.0, 10.0, -5.0, -5.0, -5.0, 5.0, 5.0, 5.0,
        ],
    }
}

/// An index of two maps, three archetypes and the bodies that draw two of them.
fn made_up(archive: PathBuf) -> WorldIndex {
    let mut second = map(22, 2);
    (second.parents_from, second.parents_len) = (0, 1);
    WorldIndex {
        archives: vec![archive],
        maps: vec![map(11, 0), second],
        parents: vec![11],
        archetypes: vec![
            archetype(300, 900, 0, 1),
            archetype(100, 800, 0, 2),
            archetype(200, 0, 700, 3),
        ],
        models: vec![body(900, 0, 0x900), body(800, 0, 0x800)],
        dictionaries: vec![body(700, 0, 0x700)],
        report: Report {
            maps: 2,
            at_rest: 1,
            ..Report::default()
        },
        ..WorldIndex::default()
    }
    .finish()
}

#[test]
fn an_archetype_finds_its_model_by_word_and_a_dictionary_s_by_its_own_name() {
    let index = made_up(PathBuf::from("levels.rpf"));
    let plain = index.archetype(100).expect("the archetype of word 100");
    assert_eq!(
        index.model(plain),
        Some(ModelRef {
            archive: 0,
            offset: 0x800,
            entry: None
        })
    );
    let in_dictionary = index.archetype(200).expect("the archetype of word 200");
    assert_eq!(
        index.model(in_dictionary),
        Some(ModelRef {
            archive: 0,
            offset: 0x700,
            entry: Some(3)
        })
    );
    assert!(index.archetype(101).is_none());
    // An interior names no model, and a word no body states draws nothing.
    assert_eq!(index.model(&archetype(1, 0, 0, 9)), None);
    assert_eq!(index.model(&archetype(1, 555, 0, 9)), None);
    assert_eq!(index.map_named(22), Some(1));
    assert_eq!(index.parents_of(&index.maps[1]), &[11]);
    // The camera starts over the maps that stream at rest alone.
    assert_eq!(index.middle(), Some(Vec3::ZERO));
}

#[test]
fn the_later_of_two_bodies_that_state_one_archetype_stands() {
    let mut index = WorldIndex {
        archetypes: vec![
            archetype(5, 1, 0, 1),
            archetype(4, 9, 0, 1),
            archetype(5, 2, 0, 1),
        ],
        ..WorldIndex::default()
    };
    index = index.finish();
    assert_eq!(index.archetype(5).map(|a| a.model), Some(2));
    assert_eq!(index.report.archetypes, 2);
}

#[test]
fn a_kept_index_reads_back_as_it_was_until_an_archive_changes() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let archive = dir.path().join("levels.rpf");
    std::fs::write(&archive, b"made up").expect("the archive");
    let index = made_up(archive.clone());
    let file = kept_at(dir.path(), dir.path());
    index.keep(&file).expect("kept");

    let read = WorldIndex::kept(&file).expect("the kept index");
    assert_eq!(read.archives, index.archives);
    assert_eq!(read.maps, index.maps);
    assert_eq!(read.parents, index.parents);
    assert_eq!(read.archetypes, index.archetypes);
    assert_eq!(read.models, index.models);
    assert_eq!(read.dictionaries, index.dictionaries);
    assert_eq!(read.report, index.report);
    assert_eq!(read.map_named(22), Some(1));

    // A kept index cut short is refused, never read as fewer rows.
    let whole = std::fs::read(&file).expect("the kept bytes");
    std::fs::write(&file, &whole[..whole.len() - 9]).expect("cut");
    assert!(WorldIndex::kept(&file).is_none());
    std::fs::write(&file, &whole).expect("whole again");
    assert!(WorldIndex::kept(&file).is_some());

    // An archive of another size is another archive: its index is built again.
    std::fs::write(&archive, b"made up, and longer").expect("the archive");
    assert!(WorldIndex::kept(&file).is_none());
}

#[test]
fn a_body_lies_in_the_last_nested_archive_whose_names_have_not_ended_it() {
    // The archive's own header at 0, one nested archive ended at 500, one that states no end.
    let archives = [(0, None), (100, Some(500)), (1000, None)];
    assert_eq!(container_of(&archives, 50), None);
    assert_eq!(container_of(&archives, 100), Some(100));
    assert_eq!(container_of(&archives, 499), Some(100));
    assert_eq!(
        container_of(&archives, 500),
        None,
        "past the names that end it"
    );
    assert_eq!(container_of(&archives, 4000), Some(1000));
    assert_eq!(container_of(&[], 4000), None);
}

#[test]
fn a_dictionary_word_is_the_one_body_that_holds_a_drawable_under_every_name() {
    let archetypes = [
        archetype(1, 0, 70, 10),
        archetype(2, 0, 70, 11),
        archetype(3, 0, 80, 12),
        archetype(4, 0, 90, 13),
        archetype(5, 500, 0, 14),
    ];
    let mut keys: HashMap<u32, Vec<BodyRow>> = HashMap::new();
    // Body 0x100 holds 10 and 11; body 0x200 holds 10 alone; 12 is held by two bodies; 13 by none.
    keys.insert(10, vec![body(0, 0, 0x100), body(0, 0, 0x200)]);
    keys.insert(11, vec![body(0, 0, 0x100)]);
    keys.insert(12, vec![body(0, 0, 0x300), body(0, 1, 0x300)]);
    let mut found = dictionaries(&archetypes, &keys);
    found.sort_unstable_by_key(|b| b.word);
    assert_eq!(found, vec![body(70, 0, 0x100)]);
}

#[test]
fn the_level_s_folders_are_counted_from_the_archives_named_on_the_platform_device() {
    let file = |name: &str| or_title::dlc::RegisteredFile {
        filename: name.to_string(),
        ..Default::default()
    };
    let files = [
        file("platform:/levels/made/up/a_metadata.rpf"),
        file("platform:/LEVELS/made/up/b.rpf"),
        file("platform:/levels/made/up/c.ityp"),
        file("platform:/textures/d.rpf"),
        file("common:/data/levels/e.rpf"),
        file("platform:/loose.rpf"),
    ];
    let refs: Vec<&or_title::dlc::RegisteredFile> = files.iter().collect();
    let folders = level_folders(&refs);
    assert_eq!(folders.get("levels"), Some(&2));
    assert_eq!(folders.get("textures"), Some(&1));
    assert_eq!(folders.len(), 2);
}

#[test]
fn what_a_range_found_before_the_end_of_the_last_body_before_it_is_dropped() {
    let mut shard = Shard {
        models: vec![(1, 90), (2, 120), (3, 300)],
        archives: vec![80, 150],
        ends: vec![(80, 100), (150, 400)],
        offsets: vec![80, 90, 120, 150, 300],
        ..Shard::default()
    };
    shard.cut_before(100);
    assert_eq!(shard.models, vec![(2, 120), (3, 300)]);
    assert_eq!(shard.archives, vec![150]);
    assert_eq!(shard.ends, vec![(150, 400)]);
    assert_eq!(shard.offsets, vec![120, 150, 300]);
}
