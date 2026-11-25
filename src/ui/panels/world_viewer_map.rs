//! The World panel's overhead map and the camera's place in words: the maps that stream at rest
//! laid on a grid at run time from the index, the camera on it, and a click to look over a point.

use std::sync::Arc;

use glam::{Vec2, Vec3};

use super::world_viewer_places::Place;
use crate::state::gta6_world::worldview::index::{Enabled, WorldIndex};
use crate::state::gta6_world::worldview::Viewer;
use crate::ui::accessible_name::named;
use crate::ui::theme;

/// The cells a side of the map is counted in.
const SIDE: usize = 256;
/// A box wider than this, a far level of detail's, is not a building: it neither lights a cell nor names a place.
const WIDEST_BOX: f32 = 400.0;
/// The tallest the map is drawn, in points, and the most of the panel's height it takes.
const MOST_HEIGHT: f32 = 280.0;
const SHARE: f32 = 0.3;
/// How far the camera moves before its place is put in words again, in metres.
const MOVED: f32 = 16.0;

/// A map as the overhead map counts it: the middle of its entities' box, its corners and its name.
pub struct Laid {
    pub low: Vec3,
    pub high: Vec3,
    pub name: u32,
}

/// The maps at rest counted on a square grid over the level.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// The south-west corner and the side of the square, in metres.
    pub origin: Vec2,
    pub side: f32,
    /// The maps whose middle lies in each cell, row 0 the northernmost.
    pub counts: Vec<u32>,
    /// The top of the tallest building box of each cell, `NEG_INFINITY` where none.
    pub tops: Vec<f32>,
}

impl Grid {
    /// The grid of `maps`, `None` where none has a finite box.
    pub fn of(maps: &[Laid]) -> Option<Self> {
        let middles = || {
            (maps.iter())
                .filter(|m| (m.high - m.low).truncate().max_element() <= WIDEST_BOX)
                .map(|m| ((m.low + m.high) / 2.0, m.high.z))
                .filter(|(middle, top)| middle.is_finite() && top.is_finite())
        };
        // ⚠ A few maps lie far out at sea or under the level: the square spans all but the outermost half percent each way.
        let (mut xs, mut ys): (Vec<f32>, Vec<f32>) = middles().map(|(m, _)| (m.x, m.y)).unzip();
        if xs.is_empty() {
            return None;
        }
        xs.sort_unstable_by(f32::total_cmp);
        ys.sort_unstable_by(f32::total_cmp);
        let (first, last) = (xs.len() / 200, xs.len() - 1 - xs.len() / 200);
        let (low, high) = (
            Vec2::new(xs[first], ys[first]),
            Vec2::new(xs[last], ys[last]),
        );
        // A square a little wider than the maps, so that none lies on its edge.
        let side = (high - low).max_element().max(1.0) * 1.04;
        let origin = (low + high) / 2.0 - Vec2::splat(side / 2.0);
        let mut grid = Self {
            origin,
            side,
            counts: vec![0; SIDE * SIDE],
            tops: vec![f32::NEG_INFINITY; SIDE * SIDE],
        };
        for (middle, top) in middles() {
            if let Some(at) = grid.cell(middle.truncate()) {
                grid.counts[at] += 1;
                grid.tops[at] = grid.tops[at].max(top);
            }
        }
        Some(grid)
    }

    /// Where `point` lies on the map, from 0 to 1 across and down.
    pub fn to_map(&self, point: Vec2) -> Vec2 {
        let along = (point - self.origin) / self.side;
        Vec2::new(along.x, 1.0 - along.y)
    }

    /// The point of the level at `at` on the map, from 0 to 1 across and down.
    pub fn to_level(&self, at: Vec2) -> Vec2 {
        self.origin + Vec2::new(at.x, 1.0 - at.y) * self.side
    }

    fn cell(&self, point: Vec2) -> Option<usize> {
        let at = self.to_map(point) * SIDE as f32;
        let (x, y) = (at.x.floor(), at.y.floor());
        let inside = (0.0..SIDE as f32).contains(&x) && (0.0..SIDE as f32).contains(&y);
        inside.then(|| y as usize * SIDE + x as usize)
    }

    /// The tallest building box within `reach` metres of `point`, where any.
    pub fn top_near(&self, point: Vec2, reach: f32) -> Option<f32> {
        let cells = (reach / self.side * SIDE as f32).ceil() as i32;
        let at = self.to_map(point) * SIDE as f32;
        let mut top = f32::NEG_INFINITY;
        for dy in -cells..=cells {
            for dx in -cells..=cells {
                let (x, y) = (at.x as i32 + dx, at.y as i32 + dy);
                if (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y) {
                    top = top.max(self.tops[y as usize * SIDE + x as usize]);
                }
            }
        }
        top.is_finite().then_some(top)
    }

    /// The maps as a mask the theme's text colour tints: a cell as opaque as the log of the maps it holds.
    fn image(&self) -> egui::ColorImage {
        let most = self.counts.iter().copied().max().unwrap_or(0).max(1) as f32;
        mask(
            self.counts
                .iter()
                .map(|&n| (1.0 + n as f32).ln() / (1.0 + most).ln() * 0.9),
        )
    }

    /// The middle of the cell at column `col`, row `row`.
    fn middle(&self, col: usize, row: usize) -> Vec2 {
        let across = |k: usize| (k as f32 + 0.5) / SIDE as f32;
        self.to_level(Vec2::new(across(col), across(row)))
    }

    /// The land and the water of each cell: the ground under its middle, and whether water stands over it there, a quad's or, past every quad, the sea's at `sea`.
    pub fn shade(&self, ground: impl Fn(Vec2) -> Option<f32>, quads: &[Water], sea: f32) -> Shade {
        let mut shade = Shade {
            ground: vec![f32::NAN; SIDE * SIDE],
            wet: vec![false; SIDE * SIDE],
        };
        for row in 0..SIDE {
            for col in 0..SIDE {
                shade.ground[row * SIDE + col] = ground(self.middle(col, row)).unwrap_or(f32::NAN);
            }
        }
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for quad in quads {
            (low, high) = (low.min(quad.low), high.max(quad.high));
            // The cells whose middles the quad may hold, north-west to south-east on the map.
            let (from, to) = (
                self.to_map(Vec2::new(quad.low.x, quad.high.y)),
                self.to_map(Vec2::new(quad.high.x, quad.low.y)),
            );
            let cells = |a: f32, b: f32| {
                let at = |t: f32| (t * SIDE as f32).clamp(0.0, SIDE as f32 - 1.0) as usize;
                at(a)..=at(b)
            };
            for row in cells(from.y, to.y) {
                for col in cells(from.x, to.x) {
                    let middle = self.middle(col, row);
                    let inside = (quad.low.cmple(middle) & middle.cmple(quad.high)).all();
                    let at = row * SIDE + col;
                    // ⚠ The ground NaN where the height map holds none: water is then taken as standing.
                    if inside && !(shade.ground[at] >= quad.z) {
                        shade.wet[at] = true;
                    }
                }
            }
        }
        for row in 0..SIDE {
            for col in 0..SIDE {
                let middle = self.middle(col, row);
                let past = !(low.cmple(middle) & middle.cmple(high)).all();
                let at = row * SIDE + col;
                shade.wet[at] |= past && shade.ground[at] < sea;
            }
        }
        shade
    }
}

/// A quad of water as the overhead map lays it: its corners and its surface's height.
pub struct Water {
    pub low: Vec2,
    pub high: Vec2,
    pub z: f32,
}

/// What the height map and the water make of each cell of a [`Grid`], row 0 the northernmost.
#[derive(Debug, Clone, PartialEq)]
pub struct Shade {
    /// The ground under the cell's middle in metres, NaN where the height map holds none.
    pub ground: Vec<f32>,
    pub wet: Vec<bool>,
}

impl Shade {
    /// The land as a mask: opaque as it stands high and as it faces the north-west, and clear under water.
    fn land(&self) -> egui::ColorImage {
        let highest = (self.ground.iter().copied())
            .filter(|h| h.is_finite())
            .fold(1.0_f32, f32::max);
        let at = |k: usize| self.ground[k];
        mask((0..SIDE * SIDE).map(|k| {
            let h = at(k);
            if self.wet[k] || !h.is_finite() {
                return 0.0;
            }
            // The rise from the cell to the north-west, lit from there.
            let behind = (k >= SIDE + 1 && k % SIDE > 0).then(|| at(k - SIDE - 1));
            let rise = behind.filter(|b| b.is_finite()).map_or(0.0, |b| h - b);
            0.2 + 0.35 * (h.max(0.0) / highest).sqrt() + (rise / 20.0).clamp(-0.12, 0.12)
        }))
    }

    /// The water as a mask.
    fn water(&self) -> egui::ColorImage {
        mask(self.wet.iter().map(|&wet| if wet { 0.55 } else { 0.0 }))
    }
}

/// A side-by-side square of cells as white of the opacity `each` gives, from 0 to 1, for a theme's colour to tint.
fn mask(each: impl Iterator<Item = f32>) -> egui::ColorImage {
    let pixels = each
        .map(|a| egui::Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0) as u8))
        .collect();
    egui::ColorImage {
        size: [SIDE, SIDE],
        source_size: egui::vec2(SIDE as f32, SIDE as f32),
        pixels,
    }
}

/// The maps of `index` that stream at rest, as the overhead map lays them.
fn laid(index: &WorldIndex) -> Vec<Laid> {
    let at_rest = Enabled::default();
    (index.maps.iter())
        .filter(|m| at_rest.allows(m))
        .map(|m| {
            let (low, high) = m.boxes().entities;
            Laid {
                low,
                high,
                name: m.name,
            }
        })
        .collect()
}

/// The point of the compass a camera turned by `yaw` degrees looks to: 0 north, 1 north-east, on to 7.
pub fn compass(yaw: f32) -> usize {
    // A growing yaw turns from north towards the west.
    let east_of_north = (-yaw).rem_euclid(360.0);
    ((east_of_north / 45.0).round() as usize) % 8
}

/// The smallest building box that holds `point` whose name `name` gives, and that name.
pub fn named_under(
    maps: &[Laid],
    point: Vec2,
    name: impl Fn(u32) -> Option<String>,
) -> Option<String> {
    let mut best: Option<(f32, u32)> = None;
    for map in maps {
        let size = (map.high - map.low).truncate();
        let holds = (map.low.truncate().cmple(point) & point.cmple(map.high.truncate())).all();
        if !holds || size.max_element() > WIDEST_BOX {
            continue;
        }
        let area = size.x * size.y;
        if best.is_some_and(|(least, _)| least <= area) {
            continue;
        }
        if name(map.name).is_some() {
            best = Some((area, map.name));
        }
    }
    best.and_then(|(_, map)| name(map))
}

/// A name as words: `region_vice_beach_name` is `Vice Beach`.
pub fn as_words(raw: &str) -> String {
    let bare = raw.strip_suffix("_name").unwrap_or(raw);
    let bare = (["region_", "location_"].iter())
        .find_map(|head| bare.strip_prefix(head))
        .unwrap_or(bare);
    let word = |w: &str| {
        let mut letters = w.chars();
        (letters.next()).map_or_else(String::new, |first| {
            first.to_uppercase().chain(letters).collect()
        })
    };
    (bare.split('_'))
        .filter(|w| !w.is_empty())
        .map(word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The smallest place of `index` holding `point` that the vocabulary names, by its label, else its own name, as words.
pub fn place_at(
    index: &WorldIndex,
    vocabulary: &or_io::gta6::names::Vocabulary,
    point: Vec2,
) -> Option<String> {
    (index.places_at(point.to_array()).into_iter()).find_map(|place| {
        let label = (place.label != 0)
            .then(|| vocabulary.name32(place.label))
            .flatten();
        label
            .or_else(|| vocabulary.name32(place.name))
            .map(as_words)
    })
}

/// What the overhead map holds between frames, by the index it was laid from.
#[derive(Default)]
pub struct Overview {
    of: usize,
    maps: Vec<Laid>,
    grid: Option<Grid>,
    texture: Option<egui::TextureHandle>,
    /// The land and the water, by the height map and the water they were shaded from.
    shaded_with: (usize, usize),
    land: Option<egui::TextureHandle>,
    water: Option<egui::TextureHandle>,
    /// Where the map under the camera was last looked for, by the vocabulary it was named with, and its name.
    named_at: Option<(Vec2, usize)>,
    name: Option<String>,
    /// The place the world-state data names there.
    pub place: Option<String>,
}

impl crate::ui::panel_state::PanelState for Overview {
    const KEY: &'static str = "world-overview";
}

impl Overview {
    /// Lay the maps of `index` again where it is another index, and shade the land and the water again once the viewer has read them.
    fn hold(&mut self, ctx: &egui::Context, index: &Arc<WorldIndex>, viewer: &Viewer) {
        let of = Arc::as_ptr(index) as usize;
        if self.of != of {
            // A pass over every map at rest, once an index.
            self.of = of;
            self.maps = laid(index);
            self.grid = Grid::of(&self.maps);
            self.texture = (self.grid.as_ref()).map(|grid| {
                ctx.load_texture("world-overview", grid.image(), egui::TextureOptions::LINEAR)
            });
            self.named_at = None;
            self.shaded_with = (0, 0);
        }
        let (ground, water) = viewer.land_and_water();
        let address = |a: Option<usize>| a.unwrap_or(0);
        let with = (
            address(ground.as_ref().map(|g| Arc::as_ptr(g) as usize)),
            address(water.as_ref().map(|w| Arc::as_ptr(w) as usize)),
        );
        let Some(grid) = self.grid.as_ref().filter(|_| with != self.shaded_with) else {
            return;
        };
        // ⚠ A pass over the grid's cells and every quad of water, once each is read.
        self.shaded_with = with;
        let quads: Vec<Water> = (water.iter().flat_map(|w| &w.quads))
            .filter(|q| !q.invisible)
            .map(|q| Water {
                low: Vec2::new(q.min[0].into(), q.min[1].into()),
                high: Vec2::new(q.max[0].into(), q.max[1].into()),
                z: q.z,
            })
            .collect();
        // The sea past the quads stands at the height the water's borders give it.
        let sea = (water.as_ref().and_then(|w| w.border.as_ref()))
            .map_or(0.0, |b| b.corners.iter().sum::<f32>() / 4.0);
        let shade = grid.shade(
            |at| ground.as_ref().and_then(|g| g.ground(at.x, at.y)),
            &quads,
            sea,
        );
        let options = egui::TextureOptions::LINEAR;
        self.land = ground
            .is_some()
            .then(|| ctx.load_texture("world-land", shade.land(), options));
        self.water = (ground.is_some() || !quads.is_empty())
            .then(|| ctx.load_texture("world-water", shade.water(), options));
    }
}

/// The overhead map: the camera on it, and a click that puts the camera to look over the point clicked.
pub fn draw(ui: &mut egui::Ui, overview: &mut Overview, viewer: &mut Viewer) {
    let Some(index) = viewer.index.clone() else {
        return;
    };
    overview.hold(ui.ctx(), &index, viewer);
    let (Some(grid), Some(texture)) = (&overview.grid, &overview.texture) else {
        return;
    };
    // ⚠ At 720 points high a map as wide as the panel leaves the panel's last sections below the fold.
    let side = (ui.available_width().min(MOST_HEIGHT)).min(ui.clip_rect().height() * SHARE);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let response = named(
        response,
        tr!("The level from above: the water, the land by its height, and the brighter, the more maps; click to look over a place"),
    );
    let painter = ui.painter_at(rect);
    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS_SMALL),
        theme::BG_L0,
    );
    for (layer, tint) in [
        (overview.land.as_ref(), theme::TEXT_SECONDARY.get()),
        (overview.water.as_ref(), theme::INFO.get()),
        (Some(texture), theme::TEXT_PRIMARY.get()),
    ] {
        if let Some(layer) = layer {
            painter.image(layer.id(), rect, whole, tint);
        }
    }
    let on_screen = |at: Vec2| rect.min + egui::vec2(at.x, at.y) * rect.size();
    let camera = &viewer.camera;
    let at = on_screen(grid.to_map(camera.position.truncate()));
    let yaw = camera.yaw;
    // Yaw 0 looks along +Y, north, up the map; a growing yaw turns towards −X.
    let ahead = egui::vec2(-yaw.sin(), -yaw.cos()) * 12.0;
    let stroke = egui::Stroke::new(2.0_f32, theme::WARNING);
    painter.line_segment([at, at + ahead], stroke);
    painter.circle_filled(at, 3.5, theme::WARNING);
    if let Some(pointer) = response
        .interact_pointer_pos()
        .filter(|_| response.clicked())
    {
        let across = (pointer - rect.min) / rect.size();
        let point = grid.to_level(Vec2::new(across.x, across.y));
        let ground = (grid.top_near(point, 256.0))
            .or_else(|| viewer.ground_under(point.x, point.y))
            .unwrap_or(0.0);
        let place = Place {
            head: None,
            from_place: false,
            maps: 0,
            middle: point.extend(ground),
        };
        place.pose().put(&mut viewer.camera);
        viewer.cuts += 1;
        ui.ctx().request_repaint();
    }
    response.on_hover_text(tr!(
        "Click to look north over a place, from 250 m south of it"
    ));
}

/// Where the camera stands, in words: the map under it where the vocabulary names one, how high over the ground and which way it looks.
pub fn place_words(overview: &mut Overview, viewer: &Viewer) -> String {
    let camera = &viewer.camera;
    let point = camera.position.truncate();
    let vocabulary = crate::state::gta6_names::vocabulary();
    let named_with = Arc::as_ptr(&vocabulary) as usize;
    // ⚠ The name is a pass over every map at rest: it is looked for again only once the camera moved or the vocabulary changed.
    let fresh =
        (overview.named_at).is_some_and(|(at, of)| at.distance(point) < MOVED && of == named_with);
    if !fresh {
        overview.name = named_under(&overview.maps, point, |hash| {
            vocabulary.name32(hash).map(str::to_string)
        });
        overview.place = (viewer.index.as_deref()).and_then(|i| place_at(i, &vocabulary, point));
        overview.named_at = Some((point, named_with));
    }
    let direction = [
        tr!("north"),
        tr!("north-east"),
        tr!("east"),
        tr!("south-east"),
        tr!("south"),
        tr!("south-west"),
        tr!("west"),
        tr!("north-west"),
    ][compass(camera.yaw.to_degrees())];
    // ⚠ The height map is the level's open ground: a camera under it, in a basement or a tunnel, is said by its height alone.
    let ground =
        (viewer.ground_under(point.x, point.y)).filter(|ground| camera.position.z >= *ground);
    let height = (camera.position.z - ground.unwrap_or(0.0)).round() as i32;
    match (overview.name.clone(), ground) {
        (Some(name), Some(_)) => tr!(
            "Over {name}, {height} m above the ground, looking {direction}",
            name = name,
            height = height,
            direction = direction
        ),
        (Some(name), None) => tr!(
            "Over {name}, at {height} m, looking {direction}",
            name = name,
            height = height,
            direction = direction
        ),
        (None, Some(_)) => tr!(
            "{height} m above the ground, looking {direction}",
            height = height,
            direction = direction
        ),
        (None, None) => tr!(
            "At {height} m, looking {direction}",
            height = height,
            direction = direction
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(x: f32, y: f32, side: f32, top: f32, name: u32) -> Laid {
        Laid {
            low: Vec3::new(x - side / 2.0, y - side / 2.0, 0.0),
            high: Vec3::new(x + side / 2.0, y + side / 2.0, top),
            name,
        }
    }

    #[test]
    fn the_overhead_map_counts_buildings_north_up_and_a_click_reads_back_the_point() {
        let maps = [
            map(0.0, 0.0, 20.0, 30.0, 1),
            map(0.0, 0.0, 10.0, 90.0, 2),
            map(1000.0, 1000.0, 20.0, 10.0, 3),
            // A far level of detail's box lights nothing and widens nothing.
            map(9000.0, 9000.0, 4000.0, 500.0, 4),
        ];
        let grid = Grid::of(&maps).expect("maps with boxes");
        assert!(grid.side > 1000.0 && grid.side < 1100.0, "{}", grid.side);
        assert_eq!(grid.counts.iter().sum::<u32>(), 3);
        assert_eq!(
            grid.counts.iter().copied().max(),
            Some(2),
            "two middles share a cell"
        );
        let south_west = grid.to_map(Vec2::ZERO);
        let north_east = grid.to_map(Vec2::splat(1000.0));
        assert!(
            south_west.x < north_east.x && south_west.y > north_east.y,
            "north is up"
        );
        let back = grid.to_level(grid.to_map(Vec2::new(250.0, 750.0)));
        assert!(back.distance(Vec2::new(250.0, 750.0)) < 0.01, "{back}");
        assert_eq!(grid.top_near(Vec2::ZERO, 50.0), Some(90.0));
        assert_eq!(grid.top_near(Vec2::new(500.0, 500.0), 50.0), None);
        assert_eq!(Grid::of(&[]), None);
    }

    #[test]
    fn a_place_s_label_is_said_as_words() {
        assert_eq!(as_words("region_vice_beach_name"), "Vice Beach");
        assert_eq!(as_words("region_key_lento"), "Key Lento");
        assert_eq!(as_words("location_247_name"), "247");
        assert_eq!(as_words("ambrosia"), "Ambrosia");
    }

    #[test]
    fn a_cell_is_wet_under_a_quad_above_its_ground_and_past_every_quad_under_the_sea() {
        let maps = [
            map(0.0, 0.0, 20.0, 30.0, 1),
            map(1000.0, 1000.0, 20.0, 10.0, 2),
        ];
        let grid = Grid::of(&maps).expect("maps with boxes");
        // A hill west of x 500, a lake bed east of it, and the sea's floor past y 900.
        let ground = |at: Vec2| {
            Some(if at.y > 900.0 {
                -20.0
            } else if at.x < 500.0 {
                40.0
            } else {
                2.0
            })
        };
        let lake = Water {
            low: Vec2::new(100.0, 100.0),
            high: Vec2::new(800.0, 400.0),
            z: 5.0,
        };
        let shade = grid.shade(ground, &[lake], 0.0);
        let at = |point: Vec2| {
            let on = grid.to_map(point) * SIDE as f32;
            on.y as usize * SIDE + on.x as usize
        };
        assert!(
            shade.wet[at(Vec2::new(700.0, 250.0))],
            "the quad stands over the lake bed"
        );
        assert!(
            !shade.wet[at(Vec2::new(200.0, 250.0))],
            "the hill stands over the quad"
        );
        assert!(
            !shade.wet[at(Vec2::new(700.0, 600.0))],
            "ground above the sea past the quads"
        );
        assert!(
            shade.wet[at(Vec2::new(200.0, 950.0))],
            "the sea past every quad"
        );
        assert_eq!(shade.ground[at(Vec2::new(200.0, 250.0))], 40.0);
        let none = grid.shade(|_| None, &[], 0.0);
        assert!(none.wet.iter().all(|w| !w) && none.ground.iter().all(|h| h.is_nan()));
    }

    #[test]
    fn the_place_is_the_smallest_named_box_under_the_camera_and_the_compass_turns_west_with_the_yaw(
    ) {
        let maps = [
            map(0.0, 0.0, 300.0, 10.0, 1),
            map(0.0, 0.0, 40.0, 10.0, 2),
            map(0.0, 0.0, 10.0, 10.0, 3),
        ];
        let name = |hash: u32| (hash != 3).then(|| format!("map{hash}"));
        assert_eq!(
            named_under(&maps, Vec2::ZERO, name).as_deref(),
            Some("map2"),
            "the smallest named"
        );
        assert_eq!(
            named_under(&maps, Vec2::new(100.0, 0.0), name).as_deref(),
            Some("map1")
        );
        assert_eq!(named_under(&maps, Vec2::new(1000.0, 0.0), name), None);
        assert_eq!(compass(0.0), 0, "a yaw of 0 looks north");
        assert_eq!(compass(90.0), 6, "a yaw of 90 looks west");
        assert_eq!(compass(270.0), 2, "east");
        assert_eq!(compass(315.0), 1, "north-east");
        assert_eq!(compass(-45.0), 1);
        assert_eq!(compass(180.0), 4);
    }
}
