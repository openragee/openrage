//! `openrage-cli rpf trace`: one scanned GTA VI drawable traced on the CPU by the reference walk
//! of `or_io::gta6::accel`, its depth printed as digits or written as a PNG.
//!
//! No device takes part: the arena holds what the game's own selection gives its ray tracer of the
//! drawable, a bottom level a model, and every ray is walked by the per-lane walk, so the picture
//! is what a lowered walk must give.

use std::collections::HashSet;
use std::path::PathBuf;

use or_io::codec::texture::{save_png, RgbaImage};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9ZlibRun};
use or_io::gta5::shaders::sgd5::Sgd5File;
use or_io::gta6::accel::select::{
    level, select_level, triangles, twice_described, Selection, Traced,
};
use or_io::gta6::accel::{walk_lane, Arena, Instance, Ray, Split, Triangle, Walk};
use or_io::gta6::headerless::{
    read, trace_words, trace_words_of_entry, Drawable, Entry, Resource, TraceWords,
};

use crate::project::Failure::{self, Run, Usage};

const USAGE: &str = "usage: rpf trace <archive.rpf> <offset> [--entry <code>] \
                     [--effects <archive.rpf> <offset>] [--level <n>] [--reduced] \
                     [--split median|area] [--size <n>] [--png <out.png>] [--limit <bytes>]";

/// The direction the picture looks along, in the model's frame, whose z is up: from in front of
/// it, to one side and above.
const FORWARD: [f32; 3] = [-0.55, 0.7, -0.45];

/// A traced picture: a depth a ray, nearest 0 and farthest 1, `None` where the ray met nothing.
struct Picture {
    width: usize,
    depth: Vec<Option<f32>>,
    arena_bytes: usize,
    /// What building the arena took.
    built: std::time::Duration,
    turns: u64,
    climbs: u64,
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / length)
}

/// A bottom level of each of `structures` placed once, unturned, and a grid of parallel rays
/// through the sphere about their boxes. A triangle that is not opaque is traced solid: its
/// texture is not read.
fn picture(
    structures: &[&[Triangle]],
    width: usize,
    height: usize,
    split: Split,
) -> Result<Picture, String> {
    let started = std::time::Instant::now();
    let mut arena = Arena::with_split(split);
    let mut placed = Vec::new();
    for (id, triangles) in structures.iter().enumerate() {
        placed.push(Instance {
            bottom: arena.bottom_level(triangles).map_err(|e| e.to_string())?,
            object_to_world: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            id: id as u32,
            mask: 0xFF,
            flags: 0,
            contribution: 0,
        });
    }
    let top = arena.top_level(&placed).map_err(|e| e.to_string())?;
    let built = started.elapsed();
    let bound = |pick: fn(f32, f32) -> f32, of: fn(&Instance) -> [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|k| {
            placed
                .iter()
                .map(|p| of(p)[k])
                .fold(of(&placed[0])[k], pick)
        })
    };
    let (min, max) = (
        bound(f32::min, |p| p.bottom.min),
        bound(f32::max, |p| p.bottom.max),
    );
    let centre: [f32; 3] = std::array::from_fn(|k| (min[k] + max[k]) / 2.0);
    let radius = (0..3)
        .map(|k| (max[k] - centre[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    let forward = unit(FORWARD);
    let right = unit(cross(forward, [0.0, 0.0, 1.0]));
    let up = cross(right, forward);
    let walk = Walk::default();
    let mut picture = Picture {
        width,
        depth: Vec::with_capacity(width * height),
        arena_bytes: arena.words().len() * 4,
        built,
        turns: 0,
        climbs: 0,
    };
    for row in 0..height {
        for column in 0..width {
            let x = ((column as f32 + 0.5) / width as f32 * 2.0 - 1.0) * radius;
            let y = (1.0 - (row as f32 + 0.5) / height as f32 * 2.0) * radius;
            // From twice the radius before the centre to twice past it: the sphere is the
            // middle half of the extent.
            let ray = Ray {
                origin: std::array::from_fn(|k| {
                    centre[k] + right[k] * x + up[k] * y - forward[k] * 2.0 * radius
                }),
                direction: forward.map(|f| f * 4.0 * radius),
                t_min: 0.0,
                t_max: 1.0,
            };
            let out = walk_lane(arena.words(), top.base, &ray, &walk, &mut |_| true);
            picture.turns += u64::from(out.turns);
            picture.climbs += u64::from(out.climbs);
            (picture.depth).push(out.hit.map(|hit| ((hit.t - 0.25) * 2.0).clamp(0.0, 1.0)));
        }
    }
    Ok(picture)
}

/// What the ray tracer is given of the drawable a body holds: the selection and its triangles.
struct Taken {
    selection: Selection,
    /// Why the selection is of its level.
    level: &'static str,
    structures: Vec<Traced>,
}

/// The name hashes of the effects described twice in the `SGD5` container at `at` of `path`.
fn twice_at(path: &str, at: u64, limit: u64) -> Result<HashSet<u32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(file, at, limit).map_err(|e| e.to_string())?;
    if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
        return Err(format!("--effects {at:#X} is not an SGD5 effect container"));
    }
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {at:#X} does not read: {e}"))?;
    Ok(twice_described(&container))
}

/// The selection of `words` and its triangles in `drawable`: of level `stated` where one is
/// given, else of the level the game takes, as for a `reduced` archetype or a plain one.
fn taken(
    words: &TraceWords,
    drawable: &Drawable,
    stated: Option<usize>,
    reduced: bool,
    twice: &HashSet<u32>,
) -> Result<Taken, String> {
    let (picked, why) = match (stated, reduced) {
        (Some(level), _) => (Some(level), "as asked"),
        (None, true) => (level(words, true), "the game's for a reduced archetype"),
        (None, false) => (level(words, false), "the game's for a plain archetype"),
    };
    let picked = picked.ok_or("no model of levels 0 to 2 is one the ray tracer draws")?;
    let selection = select_level(words, picked, |name| twice.contains(&name))
        .ok_or(format!("the drawable has no level {picked}"))?;
    let structures = triangles(&selection, drawable)
        .ok_or("the drawable's models are not those its trace words state")?;
    Ok(Taken {
        selection,
        level: why,
        structures,
    })
}

/// The drawable the body at `offset` of the archive at `path` holds, or its dictionary's `entry`,
/// with its trace words.
fn traced(
    path: &str,
    offset: u64,
    limit: u64,
    entry: Option<u32>,
) -> Result<(TraceWords, Resource), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return Err(format!(
            "the body at {offset:#X} is no headerless GTA VI resource"
        ));
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let words = match (signature.role(), entry) {
        (Some(role @ (Rpf9Role::Drawable | Rpf9Role::DrawableWithBound)), None) => {
            trace_words(role, virtual_section).pop()
        }
        (Some(Rpf9Role::Dictionary), Some(code)) => {
            trace_words_of_entry(virtual_section, code).transpose()
        }
        (Some(Rpf9Role::Dictionary), None) => {
            return Err("this is a dictionary: --entry <code> names the drawable to trace".into())
        }
        _ => return Err("this body is no drawable, fragment or dictionary entry".into()),
    };
    let words = words
        .ok_or("the dictionary holds no drawable under that code")?
        .map_err(|e| e.to_string())?;
    let read = read(signature, virtual_section, Some(physical)).map_err(|e| e.to_string())?;
    Ok((words, read))
}

/// The drawable of a read body, or its dictionary's entry `code`.
fn drawable_of(read: &Resource, entry: Option<u32>) -> Option<&Drawable> {
    match (read, entry) {
        (Resource::Drawable(drawable), None) => Some(drawable),
        (Resource::DrawableWithBound(root), None) => Some(&root.drawable),
        (Resource::Dictionary(dictionary), Some(code)) => {
            dictionary.entries.iter().find_map(|(c, e)| match e {
                Ok(Entry::Drawable(drawable)) if *c == code => Some(&**drawable),
                _ => None,
            })
        }
        _ => None,
    }
}

/// `rpf trace`: what the ray tracer is given of a scanned drawable, traced on the CPU, as digits
/// or as a PNG.
pub(crate) fn cmd_trace(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = super::rpf::split_limit(args).map_err(Usage)?;
    let (effects, args) = super::rpf::split_effects(&args).map_err(Usage)?;
    let (mut entry, mut size, mut png, mut words) = (None, None, None, Vec::new());
    let (mut stated, mut reduced, mut split) = (None, false, Split::default());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(Usage(USAGE.to_string()));
        match arg.as_str() {
            "--entry" => {
                let code = super::rpf::parse_offset(value()?).map_err(Usage)?;
                entry = Some(u32::try_from(code).map_err(|_| Usage(USAGE.to_string()))?);
            }
            "--size" => {
                let parsed = value()?
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (2..=4096).contains(n));
                size = Some(parsed.ok_or(Usage("--size takes 2 to 4096".to_string()))?);
            }
            "--level" => {
                let parsed = value()?.parse::<usize>().ok().filter(|n| *n < 4);
                stated = Some(parsed.ok_or(Usage("--level takes 0 to 3".to_string()))?);
            }
            "--reduced" => reduced = true,
            "--split" => {
                split = match value()?.as_str() {
                    "median" => Split::Median,
                    "area" => Split::SurfaceArea,
                    _ => return Err(Usage("--split takes median or area".to_string())),
                }
            }
            "--png" => png = Some(PathBuf::from(value()?)),
            _ => words.push(arg),
        }
    }
    let [path, offset] = words[..] else {
        return Err(Usage(USAGE.to_string()));
    };
    let offset = super::rpf::parse_offset(offset).map_err(Usage)?;
    let twice = match &effects {
        Some((container, at)) => twice_at(container, *at, limit)?,
        None => HashSet::new(),
    };
    let (trace, read) = traced(path, offset, limit, entry)?;
    let drawable =
        drawable_of(&read, entry).ok_or(Run("the body holds no such drawable".to_string()))?;
    let taken = taken(&trace, drawable, stated, reduced, &twice)?;
    let structures: Vec<&[Triangle]> = (taken.structures.iter())
        .map(|s| &s.triangles[..])
        .filter(|triangles| !triangles.is_empty())
        .collect();
    if structures.is_empty() {
        return Err(Run(
            "the ray tracer is given no triangle of this drawable".to_string()
        ));
    }
    // A character is about twice as tall as it is wide, so the printed grid has half the rows.
    let width = size.unwrap_or(if png.is_some() { 256 } else { 64 });
    let height = if png.is_some() { width } else { width / 2 };
    let picture = picture(&structures, width, height, split)?;
    let hits = picture.depth.iter().flatten().count();
    let models = &taken.selection.structures;
    let kept = || models.iter().flat_map(|model| &model.geometries);
    println!(
        "{path} at {offset:#X}: level {} ({}), a structure each of {} of the {} models the ray \
         tracer draws, vertices as stored (no bone moves them), traced by the reference walk on \
         the CPU",
        taken.selection.level,
        taken.level,
        structures.len(),
        models.len(),
    );
    let count = |of: fn(&Traced) -> usize| taken.structures.iter().map(of).sum::<usize>();
    println!(
        "  {} triangles of {} geometries, {} of them cutout and traced solid, {} a second \
         description ({} geometries not read, {} triangles left out), an arena of {} bytes",
        count(|s| s.triangles.len()),
        kept().count(),
        kept().filter(|g| !g.opaque).count(),
        kept().filter(|g| g.second).count(),
        count(|s| s.unread),
        count(|s| s.left_out),
        picture.arena_bytes,
    );
    match effects {
        Some(_) => println!(
            "  {} effects of the container describe their geometry twice",
            twice.len()
        ),
        None => println!("  no effect container given (--effects): no geometry is described twice"),
    }
    println!(
        "  boxes split {}, built in {:.1} ms",
        match split {
            Split::Median => "about the median",
            Split::SurfaceArea => "by surface area",
        },
        picture.built.as_secs_f64() * 1000.0,
    );
    println!(
        "  {} rays, {hits} hit, {:.1} turns a ray ({} of {}), {} climbs by the parent table",
        picture.depth.len(),
        picture.turns as f64 / picture.depth.len() as f64,
        picture.turns,
        picture.depth.len(),
        picture.climbs,
    );
    match png {
        Some(out) => {
            let grey = |depth: &Option<f32>| match depth {
                Some(depth) => [(255.0 - depth * 200.0) as u8; 3],
                None => [0; 3],
            };
            let image = RgbaImage {
                width: width as u32,
                height: height as u32,
                data: (picture.depth.iter())
                    .flat_map(|d| {
                        let [r, g, b] = grey(d);
                        [r, g, b, 255]
                    })
                    .collect(),
                is_srgb: true,
            };
            save_png(&out, &image).map_err(|e| Run(format!("write({}): {e}", out.display())))?;
            println!(
                "  -> {} ({width} by {height}, near is light)",
                out.display()
            );
        }
        None => {
            println!("  depth, 0 nearest to 9 farthest, a dot where a ray met nothing:");
            for row in picture.depth.chunks(picture.width) {
                let line: String = (row.iter())
                    .map(|depth| match depth {
                        Some(depth) => char::from(b'0' + (depth * 10.0).min(9.0) as u8),
                        None => '.',
                    })
                    .collect();
                println!("{line}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square pyramid on the ground: four sides and a floor of two triangles.
    fn pyramid() -> Vec<Triangle> {
        let base = [
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        let tip = [0.0, 0.0, 2.0];
        let mut corners: Vec<[[f32; 3]; 3]> =
            (0..4).map(|k| [base[k], base[(k + 1) % 4], tip]).collect();
        corners.extend([[base[0], base[2], base[1]], [base[0], base[3], base[2]]]);
        (corners.into_iter().enumerate())
            .map(|(primitive, corners)| Triangle {
                corners,
                geometry: 0,
                primitive: primitive as u32,
                opaque: true,
            })
            .collect()
    }

    #[test]
    fn a_pyramid_is_traced_as_its_outline_whether_it_is_one_structure_or_two() {
        let whole = pyramid();
        let (sides, floor) = whole.split_at(4);
        for structures in [vec![&whole[..]], vec![sides, floor]] {
            let picture = picture(&structures, 33, 33, Split::default()).expect("a picture");
            let at = |column: usize, row: usize| picture.depth[row * 33 + column];
            // The middle ray goes through the box's centre, inside the pyramid; the corners of
            // the picture are outside the box's sphere.
            let middle = at(16, 16).expect("the middle ray hits");
            assert!(middle > 0.0 && middle < 0.5, "{middle}");
            for (column, row) in [(0, 0), (32, 0), (0, 32), (32, 32)] {
                assert_eq!(at(column, row), None);
            }
            let hits = picture.depth.iter().flatten().count();
            assert!(hits > 33 * 33 / 8 && hits < 33 * 33 / 2, "{hits} rays hit");
        }
    }
}
