//! The GTA VI world as the viewer reads it, in digests: the index's rows, models, textures and
//! maps chosen by a rule, what is selected at the cameras of the reference script, the lights and
//! occluders read there, and the day's frame values at its hours, written to a file or held to one.
//!
//! ⚠ No digest is kept in the tree. A reference is written from the revision before a change
//! ([`WRITE_ENV`]) and the world is held to it after ([`HELD_ENV`]); a reference that is not
//! there fails.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::Vec3;
use or_io::gta6::hash::literal_hash64;
use or_job::{Cancel, Progress};
use or_renderer::game_shaders::lighting::Day;
use or_title::gta6_day::{DayTables, Moment};
use or_title::gta6_mount::Mode;
use or_world::streaming::View;

use super::index::{self, Enabled, ModelRef, WorldIndex};
use super::sky::Weathers;
use super::stream::{self, Selected, Streamer};
use super::texture::{self, OpenPacks, Places};
use super::{lights, lights_corona, lights_distant, lights_frame, model, sky, water, Viewer};
use crate::state::drivers::WeatherState;
use crate::ui::panels::world_viewer::Pose;

/// The script whose cameras, hours, modes and weathers the selections, the lights and the day are digested at.
const SCRIPT: &str = include_str!("../../../scripts/shots/gta6_world_reference.shots");

/// The folder of the GTA VI build.
const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// Names the file a reference is written to; a file that is there is not written over.
const WRITE_ENV: &str = "OPENRAGE_GTA6_WORLD_DIGESTS_WRITE";
/// Names the reference the world is held to.
const HELD_ENV: &str = "OPENRAGE_GTA6_WORLD_DIGESTS";

/// The first line of a reference file.
const HEAD: &str = "# GTA VI world reference digests 1";
/// The hour and the modes a script starts at, as the viewer does.
const NOON: f32 = 12.0;
/// How many archetypes' models are digested, evenly spaced among those the index places, in the order of their words.
const MODELS: usize = 256;
/// How many of the packs' textures are digested at [`SEEN_AT`] pixels, evenly spaced in the order of their keys; one in [`WHOLE_EVERY`] of them at every level too.
const TEXTURES: usize = 192;
const SEEN_AT: u32 = 256;
const WHOLE_EVERY: usize = 4;
/// How many of the textures the digested models name are digested, evenly spaced in the order of their keys.
const NAMED_TEXTURES: usize = 64;
/// How many maps a selection is digested over, evenly spaced among those that stream at rest: the camera stands at the middle of the map's entities, [`OVER_A_MAP`] metres over their top.
const MAPS: usize = 24;
const OVER_A_MAP: f32 = 10.0;
/// How far about a camera the water's mesh is digested, in metres.
const WATER_REACH: f32 = 2048.0;
/// How many places along each side of its extent the ground under the water is digested at, evenly spaced.
const GROUND_PLACES: u32 = 1024;
/// The frames the wind is stepped before it is digested, a sixtieth of a second each.
const WIND_FRAMES: usize = 64;
/// How long a read on workers is waited for.
const DEADLINE: Duration = Duration::from_secs(900);
/// The most selections a fade in time takes to end: a step is a fifth.
const FADE_STEPS: usize = 16;

/// One shot of the script: where the camera stands and what the world is set to.
#[derive(Debug, Clone, PartialEq)]
struct Stop {
    name: String,
    pose: Pose,
    hour: f32,
    modes: u8,
    weather: WeatherState,
}

/// The shots of `script`, each with the camera, the hour, the modes and the weather set before it; a step that sets anything else is refused.
fn stops(script: &str) -> Result<Vec<Stop>, String> {
    let (mut pose, mut hour, mut modes) = (None, NOON, Mode::Base.bit());
    let mut weather = WeatherState::Clear;
    let mut out = Vec::new();
    for (at, line) in script.lines().enumerate() {
        let line = line.trim();
        let wrong = |why: &str| format!("line {}: {why}: `{line}`", at + 1);
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["world", "camera", ..] => pose = Some(Pose::read(line).map_err(|why| wrong(&why))?),
            ["world", "hour", stated] => {
                hour = stated.parse().map_err(|_| wrong("no hour"))?;
            }
            ["world", "modes", ref named @ ..] => {
                modes = 0;
                for name in named {
                    modes |= match *name {
                        "base" => Mode::Base.bit(),
                        "story" => Mode::Story.bit(),
                        "minigames" => Mode::Minigames.bit(),
                        _ => return Err(wrong("no such mode")),
                    };
                }
            }
            ["world", "weather", name] => {
                weather = match name {
                    "clear" => WeatherState::Clear,
                    "cloudy" => WeatherState::Cloudy,
                    "rain" => WeatherState::Rain,
                    "storm" => WeatherState::Storm,
                    "fog" => WeatherState::Fog,
                    _ => return Err(wrong("no such weather")),
                };
            }
            ["shot", name] => out.push(Stop {
                name: name.to_string(),
                pose: pose.ok_or_else(|| wrong("a shot before a camera"))?,
                hour,
                modes,
                weather,
            }),
            // What sets nothing a digest is taken of: the window, the build's folder, the waits and the dump.
            []
            | ["size" | "game" | "workspace", ..]
            | ["world", "clean" | "settle" | "converge" | "dump", ..] => {}
            [first, ..] if first.starts_with('#') => {}
            // ⚠ Any other step may change what the shot after it shows, a group switched or a flight: the digests would be taken of another world than the shots.
            _ => return Err(wrong("a step the digests do not follow")),
        }
    }
    Ok(out)
}

/// The digest of `bytes`, as sixteen hexadecimal digits.
fn hex(bytes: &[u8]) -> String {
    format!("{:016x}", literal_hash64(bytes))
}

/// The digest of `records` whatever order they come in, and how many there are.
// ⚠ What a selection or a frame's lights hold comes in the order workers and hash maps gave it, which is no part of what draws.
fn of_records(mut records: Vec<Vec<u8>>) -> String {
    records.sort_unstable();
    let count = records.len();
    let mut all = Vec::new();
    for record in records {
        all.extend((record.len() as u64).to_le_bytes());
        all.extend(record);
    }
    format!("{} n={count}", hex(&all))
}

/// [`of_records`] of each of `things` as its `Debug` text says it.
fn of_debug<T: std::fmt::Debug>(things: impl IntoIterator<Item = T>) -> String {
    of_records(
        (things.into_iter())
            .map(|thing| format!("{thing:?}").into_bytes())
            .collect(),
    )
}

/// `most` of `rows` at most, evenly spaced from the first.
fn evenly<T>(rows: &[T], most: usize) -> Vec<&T> {
    let count = rows.len().min(most);
    (0..count).map(|k| &rows[k * rows.len() / count]).collect()
}

/// The digests of a world, by key.
type Digests = BTreeMap<String, String>;

/// Write `digests` at `file` as a reference, a key and its digest a line; a file that is there is refused.
fn write(file: &Path, digests: &Digests) -> Result<(), String> {
    if file.exists() {
        return Err(format!(
            "{} is there already: a reference is not written over, so that none is replaced by the world it is to judge",
            file.display()
        ));
    }
    let mut text = format!("{HEAD}\n");
    for (key, digest) in digests {
        text.push_str(&format!("{key}\t{digest}\n"));
    }
    std::fs::write(file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

/// The reference at `file`; an error where it is not there or is not one.
fn read(file: &Path) -> Result<Digests, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("no reference at {}: {e}", file.display()))?;
    let mut lines = text.lines();
    if lines.next() != Some(HEAD) {
        return Err(format!(
            "{} is no reference: it does not start with `{HEAD}`",
            file.display()
        ));
    }
    let mut digests = Digests::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, digest) = line
            .split_once('\t')
            .ok_or_else(|| format!("{}: `{line}` is no key and digest", file.display()))?;
        digests.insert(key.to_string(), digest.to_string());
    }
    if digests.is_empty() {
        return Err(format!("{} holds no digest", file.display()));
    }
    Ok(digests)
}

/// Every way `found` differs from `reference`: a digest that is another, one the reference lacks and one the world no longer gives.
fn differences(found: &Digests, reference: &Digests) -> Vec<String> {
    let keys: BTreeSet<&String> = found.keys().chain(reference.keys()).collect();
    let mut out = Vec::new();
    for key in keys {
        match (found.get(key), reference.get(key)) {
            (Some(ours), Some(theirs)) if ours == theirs => {}
            (Some(ours), Some(theirs)) => {
                out.push(format!("{key}: {ours} where the reference holds {theirs}"));
            }
            (Some(ours), None) => out.push(format!("{key}: {ours}, which the reference lacks")),
            (None, Some(theirs)) => {
                out.push(format!("{key}: nothing where the reference holds {theirs}"));
            }
            (None, None) => {}
        }
    }
    out
}

/// What was asked of the install-gated suite.
#[derive(Debug, PartialEq)]
enum Asked {
    /// Nothing: a variable it needs is not set, and it says which.
    Nothing(String),
    Write(PathBuf, PathBuf),
    Hold(PathBuf, PathBuf),
}

/// What the variables ask: the build's folder with a file to write a reference to or one to hold the world to, never both.
fn asked(variable: &dyn Fn(&str) -> Option<PathBuf>) -> Result<Asked, String> {
    let Some(root) = variable(BUILD_ENV) else {
        return Ok(Asked::Nothing(format!(
            "set {BUILD_ENV}, and {WRITE_ENV} or {HELD_ENV}"
        )));
    };
    match (variable(WRITE_ENV), variable(HELD_ENV)) {
        (None, None) => Ok(Asked::Nothing(format!(
            "set {WRITE_ENV} to write a reference or {HELD_ENV} to hold the world to one"
        ))),
        (Some(_), Some(_)) => Err(format!(
            "{WRITE_ENV} and {HELD_ENV} are both set: a run writes a reference or is held to one"
        )),
        (Some(file), None) => Ok(Asked::Write(root, file)),
        (None, Some(file)) => Ok(Asked::Hold(root, file)),
    }
}

/// Call `ready` every few milliseconds until it gives something, for [`DEADLINE`] at most.
fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let until = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = ready() {
            return found;
        }
        assert!(
            Instant::now() < until,
            "{what} was not read after {} s",
            DEADLINE.as_secs()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A camera at `pose`.
fn camera_at(pose: &Pose) -> or_renderer::FlyCamera {
    let mut camera = Viewer::default().camera;
    pose.put(&mut camera);
    camera
}

/// Stream the maps and the occluders `view` wants until none is waited for, then select with every model on the device until no fade in time is under way.
fn settled(streamer: &mut Streamer, view: &View) -> Selected {
    let mut quiet = 0;
    wait_for("the maps around a camera", || {
        let changed = streamer.stream(view);
        quiet = match changed || streamer.waiting() > 0 {
            true => 0,
            false => quiet + 1,
        };
        (quiet >= 2).then_some(())
    });
    for _ in 0..FADE_STEPS {
        let selected = streamer.select(view, &|_| Some(true));
        if !selected.fading {
            return selected;
        }
    }
    panic!("a fade in time had not ended after {FADE_STEPS} selections");
}

/// What tells one archive of `index` from another in any run: the digest of its path under `root`.
fn archive_words(index: &WorldIndex, root: &Path) -> Vec<u64> {
    (index.archives.iter())
        .map(|path| {
            let under = path.strip_prefix(root).unwrap_or(path);
            literal_hash64(under.to_string_lossy().replace('\\', "/").as_bytes())
        })
        .collect()
}

fn put_floats(record: &mut Vec<u8>, floats: &[f32]) {
    for float in floats {
        record.extend(float.to_le_bytes());
    }
}

/// `model` as a record: its archive's word, its body's offset and its entry.
fn put_model(record: &mut Vec<u8>, archives: &[u64], model: ModelRef) {
    let archive = archives.get(model.archive as usize).copied();
    record.extend(archive.unwrap_or(u64::MAX).to_le_bytes());
    record.extend(model.offset.to_le_bytes());
    record.extend(model.entry.map_or(u64::MAX, u64::from).to_le_bytes());
}

/// Digest what `selected` draws under `prefix`: each instance with its model, where it stands and how opaque, each node that draws, the census, the cards, the occluders and the room the camera stands in.
fn digest_selection(out: &mut Digests, prefix: &str, selected: &Selected, archives: &[u64]) {
    let instances = (selected.instances.iter())
        .map(|instance| {
            let mut record = Vec::new();
            match selected.models.get(instance.model as usize) {
                Some(model) => put_model(&mut record, archives, *model),
                None => record.extend([0xFF; 24]),
            }
            put_floats(&mut record, instance.rows.as_flattened());
            put_floats(&mut record, &instance.centre.to_array());
            put_floats(&mut record, &[instance.radius, instance.fade]);
            put_floats(&mut record, &instance.tint);
            put_floats(&mut record, &instance.ambient);
            record.push(u8::from(instance.cast_only));
            record
        })
        .collect();
    out.insert(format!("{prefix}.instances"), of_records(instances));
    let census = &selected.census;
    out.insert(
        format!("{prefix}.nodes"),
        of_debug(
            census
                .drawn_nodes
                .iter()
                .map(|(map, at, fade)| (map, at, fade.to_bits())),
        ),
    );
    let sorted = |names: &[u32]| {
        let mut names = names.to_vec();
        names.sort_unstable();
        names
    };
    let counted = (
        sorted(&census.wanted),
        sorted(&census.loaded),
        census.nodes,
        census.drawn,
        census.list_instances,
        selected.swaps.len(),
    );
    out.insert(
        format!("{prefix}.census"),
        format!(
            "{} wanted={} loaded={} drawn={}",
            hex(format!("{counted:?}").as_bytes()),
            census.wanted.len(),
            census.loaded.len(),
            census.drawn_nodes.len()
        ),
    );
    out.insert(format!("{prefix}.cards"), of_debug(selected.cards.iter()));
    out.insert(
        format!("{prefix}.occluders"),
        of_debug(selected.occluders.iter()),
    );
    let inside = (selected.inside, &selected.outside_through);
    out.insert(
        format!("{prefix}.inside"),
        format!(
            "{} inside={}",
            hex(format!("{inside:?}").as_bytes()),
            selected.inside.is_some()
        ),
    );
}

/// The moment of the day `tables` state, worked out for these and no other: [`sky::moment`] shows the last moment near it while a worker works a new one out.
fn exact_moment(
    tables: &Arc<DayTables>,
    hour: f32,
    weathers: Weathers,
    y: f32,
    room: Option<sky::Room>,
) -> Option<Moment> {
    wait_for("a moment of the day", || {
        let _ = sky::moment(tables, hour, weathers, y, room);
        (!sky::adapting()).then(|| sky::moment(tables, hour, weathers, y, room))
    })
}

/// The day of `moment` with the sky's radiance worked out for it and no other: [`sky::day_of_moment`] shows the last table while a worker works a new one out.
fn exact_day(tables: &DayTables, moment: &Moment, hour: f32, weathers: Weathers, eye: Vec3) -> Day {
    let day = || sky::day_of_moment(tables, moment.clone(), (hour, 0.0), weathers, eye);
    wait_for("the sky's radiance", || {
        let _ = day();
        (!sky::radiance_pending()).then(day)
    })
}

/// Digest the frame's lights at `stop` under `prefix`, once the lights of the maps about its camera are read: each volume, sprite, corona and lamp, the rooms' boxes and what was read.
fn digest_lights(
    out: &mut Digests,
    prefix: &str,
    lights: &mut lights::Lights,
    (stop, enabled): (&Stop, &Enabled),
    (tables, moment): (&DayTables, Option<&Moment>),
    selected: &Selected,
) {
    let camera = camera_at(&stop.pose);
    let settings = &tables.visual_settings;
    lights.set_reach(lights_frame::LodReach::of(settings));
    lights.set_distant_reach(lights_distant::DistantReach::of(settings));
    lights.set_corona_reach(lights_corona::CoronaReach::of(settings));
    lights.set_enabled(enabled);
    let mut quiet = 0;
    wait_for("the lights of the maps around a camera", || {
        let changed = lights.look(camera.position);
        quiet = match changed || lights.waiting() > 0 {
            true => 0,
            false => quiet + 1,
        };
        (quiet >= 2).then_some(())
    });
    let table = tables.executable.variables.as_ref();
    let frame = lights::LightFrame {
        vars: (table.zip(moment)).map_or_else(Default::default, |(table, moment)| {
            lights::LightVars::of(table, &moment.own)
        }),
        day_phase: moment.map(|moment| moment.day_phase),
    };
    let view = lights_corona::View {
        eye: camera.position,
        forward: camera.forward(),
        tan_half: (camera.fov_y * 0.5).tan(),
    };
    let scale = sky::scale_of(moment);
    let lit = lights.lights(
        &view,
        (stop.hour, scale, lights_frame::MOST),
        &selected.rooms_seen,
        &frame,
    );
    out.insert(format!("{prefix}.points"), of_debug(lit.points.iter()));
    out.insert(format!("{prefix}.spots"), of_debug(lit.spots.iter()));
    out.insert(format!("{prefix}.capsules"), of_debug(lit.capsules.iter()));
    out.insert(format!("{prefix}.sprites"), of_debug(lit.sprites.iter()));
    out.insert(format!("{prefix}.lamps"), of_debug(lit.lamps.iter()));
    out.insert(format!("{prefix}.kept"), of_debug(lit.kept.iter()));
    let (look, coronas) = (lit.coronas.as_ref()).map_or((None, &[][..]), |(look, coronas)| {
        (Some(look), coronas.as_slice())
    });
    out.insert(
        format!("{prefix}.coronas"),
        format!(
            "{} look={}",
            of_debug(coronas.iter()),
            hex(format!("{look:?} {:?}", lit.sprite_look).as_bytes())
        ),
    );
    out.insert(
        format!("{prefix}.rooms"),
        format!("n={} lights={}", lit.rooms.len(), lit.lights),
    );
    out.insert(
        format!("{prefix}.read"),
        format!("{:?}", lights.report()).replace(['{', '}', ','], ""),
    );
}

/// Digest the day's frame values at `stop` under `prefix`: the moment, the day the frame is lit by, its sky, the water's medium, waves and mesh about the camera, and the wind after [`WIND_FRAMES`] frames.
fn digest_look(
    out: &mut Digests,
    prefix: &str,
    root: &Path,
    stop: &Stop,
    (tables, moment): (&Arc<DayTables>, &Moment),
) {
    let eye = stop.pose.position;
    let weathers = Weathers::steady(stop.weather);
    out.insert(
        format!("{prefix}.moment"),
        format!(
            "{} light_scale={:?} stops={:?}",
            hex(format!("{moment:?}").as_bytes()),
            moment.light_scale,
            moment.stops
        ),
    );
    let mut day = exact_day(tables, moment, stop.hour, weathers, eye);
    let sky_view = day.sky_view.take();
    out.insert(format!("{prefix}.day"), hex(format!("{day:?}").as_bytes()));
    out.insert(
        format!("{prefix}.sky"),
        hex(format!("{sky_view:?}").as_bytes()),
    );

    // ⚠ The water's medium and waves ask for the moment out of doors, whatever room the camera stands in.
    let outdoors = exact_moment(tables, stop.hour, weathers, eye.y, None);
    let seen = water::seen(root, stop.hour, weathers, eye.y);
    let medium = seen.as_ref().and_then(water::Seen::medium);
    let waves = seen.as_ref().map(|seen| seen.waves(eye.x, eye.y));
    let mesh = seen.as_ref().map(|seen| {
        let ground = seen.ground.as_deref();
        water::mesh(&seen.quads.quads, eye, WATER_REACH, ground)
    });
    let (vertices, indices) = mesh.unwrap_or_default();
    let water = (
        outdoors.is_some(),
        medium,
        water::tint(medium.as_ref()),
        waves,
    );
    out.insert(
        format!("{prefix}.water"),
        format!(
            "{} mesh={} vertices={} indices={}",
            hex(format!("{water:?}").as_bytes()),
            hex(&[
                bytemuck::cast_slice::<_, u8>(&vertices),
                bytemuck::cast_slice::<_, u8>(&indices)
            ]
            .concat()),
            vertices.len(),
            indices.len()
        ),
    );

    // The wind runs on the game's clock, a sixtieth of a second a frame from nought.
    let mut viewer = Viewer {
        root: Some(root.to_path_buf()),
        hour: stop.hour,
        ..Viewer::default()
    };
    viewer.camera.position = eye;
    or_transport::clock::restart_game();
    let mut wind = Vec::new();
    for _ in 0..WIND_FRAMES {
        or_transport::clock::tick();
        wind.push(viewer.wind(weathers));
    }
    out.insert(
        format!("{prefix}.wind"),
        hex(format!("{wind:?}").as_bytes()),
    );
}

/// Digest the models of [`MODELS`] archetypes, each its vertex and index bytes and its material records; the texture keys their materials name.
fn digest_models(out: &mut Digests, index: &WorldIndex, places: &Places) -> BTreeSet<u64> {
    let placed: Vec<_> = (index.archetypes().iter())
        .filter_map(|row| Some((row.word, index.model(row)?)))
        .collect();
    let (mut files, mut named) = (HashMap::<u32, File>::new(), BTreeSet::new());
    for (word, model) in evenly(&placed, MODELS) {
        let key = format!("model.{word:016x}");
        let Some(read) = model::read_model(&mut files, index, Some(places), *model) else {
            out.insert(key, "unread".into());
            continue;
        };
        let (mut geometry, mut material) = (Vec::new(), String::new());
        let (mut meshes, mut vertices, mut indices) = (0, 0, 0);
        for lod in &read.lods {
            material.push_str(&format!("until {:?}\n", lod.until));
            for mesh in &lod.meshes {
                geometry.extend((mesh.vertices.len() as u64).to_le_bytes());
                // A vertex is its position, its normal, its coordinate, its colour and its depth bias, in that order.
                for vertex in &mesh.vertices {
                    put_floats(&mut geometry, &vertex.position);
                    put_floats(&mut geometry, &vertex.normal);
                    put_floats(&mut geometry, &vertex.uv);
                    put_floats(&mut geometry, &vertex.colour);
                    put_floats(&mut geometry, &[vertex.depth_bias]);
                }
                geometry.extend((mesh.indices.len() as u64).to_le_bytes());
                geometry.extend(bytemuck::cast_slice::<_, u8>(&mesh.indices));
                material.push_str(&format!("{:?} {:?}\n", mesh.bucket, mesh.material));
                named.extend(mesh.material.keys());
                meshes += 1;
                vertices += mesh.vertices.len();
                indices += mesh.indices.len();
            }
        }
        material.push_str(&format!("{:?}", read.palettes));
        out.insert(
            key,
            format!(
                "geometry={} material={} lods={} meshes={meshes} vertices={vertices} indices={indices}",
                hex(&geometry),
                hex(material.as_bytes()),
                read.lods.len()
            ),
        );
    }
    named
}

/// Digest the level bytes of the texture `key` read for a screen size of `pixels`.
fn digest_texture(out: &mut Digests, (open, places): (&OpenPacks, &Places), key: u64, pixels: u32) {
    let read = texture::read_texture(open, places, key, pixels);
    let digest = read.map_or_else(
        || "unread".to_string(),
        |read| {
            let mut bytes = Vec::new();
            for level in &read.levels {
                bytes.extend((level.len() as u64).to_le_bytes());
                bytes.extend(level);
            }
            format!(
                "{} {}x{} {:?} levels={}",
                hex(&bytes),
                read.width,
                read.height,
                read.format,
                read.levels.len()
            )
        },
    );
    out.insert(format!("texture.{key:016x}.{pixels}"), digest);
}

/// Every digest of the world of the install at `root`.
fn digests(root: &Path) -> Digests {
    let mut out = Digests::new();
    let began = Instant::now();
    let said = |what: &str| eprintln!("  {:>6.1} s  {what}", began.elapsed().as_secs_f32());
    let script = stops(SCRIPT).expect("the reference script states its shots");
    assert!(!script.is_empty(), "the reference script takes no shot");

    // The index, built afresh as the viewer builds it.
    let built = index::build(root, None, &Cancel::new(), &Progress::new());
    let index = Arc::new(
        built
            .expect("the index is built")
            .expect("no build is cancelled"),
    );
    for (name, rows, bytes) in index.tables() {
        out.insert(
            format!("index.{name}"),
            format!("{} rows={rows}", hex(bytes)),
        );
    }
    let archives = archive_words(&index, root);
    out.insert(
        "index.archives".into(),
        format!(
            "{} n={}",
            hex(bytemuck::cast_slice(&archives)),
            archives.len()
        ),
    );
    said("the index");

    // The packs, as the viewer's readers are given them.
    let (packs, keys, formats, dictionaries) =
        crate::state::gta6_texture_index::pack_places(root)(&Cancel::new(), &Progress::new())
            .expect("the texture packs are indexed");
    let places = Places {
        packs,
        keys,
        formats,
        dictionaries,
    };
    let held: Vec<(u64, u64, u16)> = {
        let pack = |at: &u32| {
            let path = places.packs.get(*at as usize);
            path.map_or(u64::MAX, |path| {
                let under = path.strip_prefix(root).unwrap_or(path);
                literal_hash64(under.to_string_lossy().replace('\\', "/").as_bytes())
            })
        };
        let mut held: Vec<_> = (places.keys.iter())
            .map(|(key, at)| {
                (
                    *key,
                    pack(at),
                    places.formats.get(key).copied().unwrap_or(0),
                )
            })
            .collect();
        held.sort_unstable();
        held
    };
    out.insert(
        "textures.packs".into(),
        format!(
            "{} keys={} packs={} dictionaries={}",
            hex(format!("{held:?}").as_bytes()),
            held.len(),
            places.packs.len(),
            places.dictionaries.1.len()
        ),
    );
    said("the texture packs");

    let named = digest_models(&mut out, &index, &places);
    said("the models");

    let open = OpenPacks::default();
    let of_packs: Vec<u64> = held.iter().map(|(key, _, _)| *key).collect();
    for (at, key) in evenly(&of_packs, TEXTURES).into_iter().enumerate() {
        digest_texture(&mut out, (&open, &places), *key, SEEN_AT);
        if at % WHOLE_EVERY == 0 {
            digest_texture(&mut out, (&open, &places), *key, u32::MAX);
        }
    }
    let named: Vec<u64> = named.into_iter().collect();
    for key in evenly(&named, NAMED_TEXTURES) {
        digest_texture(&mut out, (&open, &places), *key, SEEN_AT);
    }
    said("the textures");

    // The day's tables and the water, each read once on a worker.
    let tables = wait_for("the day's tables", || sky::tables(root));
    let reading = Viewer {
        root: Some(root.to_path_buf()),
        ..Viewer::default()
    };
    // ⚠ The ground, the fog and the waves are asked for once the quads are read: each part is seen as it stands after the last has arrived.
    let seen = || water::seen(root, NOON, Weathers::steady(WeatherState::Clear), 0.0);
    let mut quiet = 0;
    wait_for("the water", || {
        let _ = seen();
        quiet = match reading.water_reading() {
            true => 0,
            false => quiet + 1,
        };
        (quiet >= 2).then_some(())
    });
    let water = seen();
    out.insert(
        "water.quads".into(),
        water.as_ref().map_or("unread".into(), |seen| {
            format!(
                "{} quads={}",
                hex(format!("{:?}", seen.quads).as_bytes()),
                seen.quads.quads.len()
            )
        }),
    );
    // ⚠ The ground holds its tiles in a hash map, whose `Debug` text comes in another order each run: it is digested by what it answers.
    out.insert(
        "water.ground".into(),
        (water.as_ref().and_then(|seen| seen.ground.as_ref())).map_or("unread".into(), |ground| {
            let (west, north, size) = ground.extent();
            let mut heights = Vec::new();
            put_floats(&mut heights, &[west, north, size[0], size[1]]);
            let at = |k: u32, side: f32| side * (k as f32 + 0.5) / GROUND_PLACES as f32;
            for (row, column) in
                (0..GROUND_PLACES).flat_map(|r| (0..GROUND_PLACES).map(move |c| (r, c)))
            {
                let height = ground.ground(west + at(column, size[0]), north - at(row, size[1]));
                put_floats(&mut heights, &[height.unwrap_or(f32::NAN)]);
            }
            format!(
                "{} tiles={} places={GROUND_PLACES}x{GROUND_PLACES}",
                hex(&heights),
                ground.tiles()
            )
        }),
    );
    out.insert(
        "water.presets".into(),
        (water.as_ref().and_then(|seen| seen.presets.as_ref()))
            .map_or("unread".into(), |presets| {
                hex(format!("{presets:?}").as_bytes())
            }),
    );
    said("the day's tables and the water");

    // The selection, the lights and the day at each shot of the script, in its order.
    let fade = stream::fade_of(&tables.visual_settings);
    let mut streamer = Streamer::new(Arc::clone(&index));
    let mut lights = lights::Lights::new(Arc::clone(&index));
    for stop in &script {
        let camera = camera_at(&stop.pose);
        let enabled = Enabled {
            modes: stop.modes,
            groups: index.default_on_in(stop.hour, stop.modes, false),
        };
        streamer.set_enabled(enabled.clone());
        streamer.set_hour(stop.hour);
        let mut view = View::at(
            camera.position,
            or_world::streaming::camera_scale(camera.fov_y),
        );
        view.fade = fade;
        let selected = settled(&mut streamer, &view);
        let prefix = format!("shot.{}", stop.name);
        digest_selection(
            &mut out,
            &format!("{prefix}.selection"),
            &selected,
            &archives,
        );

        let weathers = Weathers::steady(stop.weather);
        let room = (selected.inside)
            .map(|inside| ([inside.timecycle, inside.secondary_timecycle], inside.blend));
        let moment = exact_moment(&tables, stop.hour, weathers, camera.position.y, room);
        digest_lights(
            &mut out,
            &format!("{prefix}.lights"),
            &mut lights,
            (stop, &enabled),
            (&tables, moment.as_ref()),
            &selected,
        );
        match &moment {
            Some(moment) => {
                let look = format!("{prefix}.look");
                digest_look(&mut out, &look, root, stop, (&tables, moment));
            }
            None => {
                out.insert(format!("{prefix}.look"), "no moment".into());
            }
        }
        said(&prefix);
    }

    // The selection over each of [`MAPS`] maps that stream at rest, at noon in the base game.
    let at_rest = Enabled::default();
    streamer.set_enabled(at_rest.clone());
    streamer.set_hour(NOON);
    let resting: Vec<_> = (index.maps.iter())
        .filter(|map| at_rest.allows(map))
        .collect();
    for map in evenly(&resting, MAPS) {
        let (low, high) = map.boxes().entities;
        let eye = ((low + high) / 2.0).with_z(high.z + OVER_A_MAP);
        let key = format!("map.{:08x}.selection", map.name);
        if !eye.is_finite() {
            out.insert(key, "no place".into());
            continue;
        }
        let mut view = View::at(eye, 1.0);
        view.fade = fade;
        let selected = settled(&mut streamer, &view);
        digest_selection(&mut out, &key, &selected, &archives);
    }
    said("the maps");
    out
}

#[path = "reference_parts_tests.rs"]
mod parts;

/// The digests of the build, written or held: the one test that reads the install.
mod of_the_install {
    use super::*;

    const WHAT: &str = "the GTA VI world reads as its reference says";

    #[test]
    fn the_world_reads_as_its_reference_says() {
        let variable = |name: &str| {
            (std::env::var_os(name).filter(|value| !value.is_empty())).map(PathBuf::from)
        };
        let asked = asked(&variable).unwrap_or_else(|why| panic!("{why}"));
        let (root, file, writes) = match asked {
            Asked::Nothing(why) => {
                or_test_support::skip_install(WHAT, why);
                return;
            }
            Asked::Write(root, file) => (root, file, true),
            Asked::Hold(root, file) => (root, file, false),
        };
        // ⚠ Judged before the world is read: a reference that is not there fails at once, and so does one that would be written over.
        let reference = match writes {
            true => {
                assert!(
                    !file.exists(),
                    "{} is there already: a reference is not written over",
                    file.display()
                );
                None
            }
            false => Some(read(&file).unwrap_or_else(|why| panic!("{why}"))),
        };
        assert!(
            index::keys().is_some(),
            "no GTA VI key snapshot reads: set {} or {}",
            or_io::container::rpf::GTA6_SNAPSHOT_ENV,
            or_io::container::rpf::GTA6_EXE_ENV
        );
        // ⚠ The clock is fixed for the whole process: under nextest this test is its own.
        or_transport::clock::fix(Duration::from_nanos(16_666_667), 0x5EED);
        let found = digests(&root);
        let Some(reference) = reference else {
            write(&file, &found).unwrap_or_else(|why| panic!("{why}"));
            eprintln!("wrote {} digests to {}", found.len(), file.display());
            return;
        };
        let differ = differences(&found, &reference);
        assert!(
            differ.is_empty(),
            "{} of {} digests differ from {}:\n{}",
            differ.len(),
            found.len().max(reference.len()),
            file.display(),
            differ.join("\n")
        );
    }
}
