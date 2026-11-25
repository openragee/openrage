//! The waves of the GTA VI world's water: the ocean simulation's presets and its three cascades,
//! each read once on a worker and held, and the sum of waves the surface is drawn with for the day.
//!
//! The sea's spectrum is the game's: JONSWAP for the wind the first cascade states at the day's
//! Beaufort number over the day's fetch, its peak sharpened by 3.3, each cascade a band of numbers
//! set by its patch's side, spread by its own blend of even and the cosine squared. ⚠ Still
//! assumed: where the second and third cascades' waves travel (the game takes the weather's wind),
//! and how the twelve waves sample the spectrum. The presets give each octave's strength along the
//! wind and pick, by the nearest place each was set up at, the kind of water.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use or_io::container::rpf::{read_rpf9_body, Gta6Mount, GtaViKeys};
use or_io::gta6::water::waves::{OceanPreset, OceanPresets, WaveParams};
use or_renderer::world::water::WAVES;
use or_title::gta6_mount::table::entry_offset;

use super::ground::{held, reading, Slots};

/// The archive and the document that state the presets.
const ARCHIVE: &str = "common.rpf";
const ENTRY: &str = "data/levels/americas/water/oceansimpresets.xml";
const BODY_LIMIT: u64 = 1 << 20;

/// Metres a second squared.
const GRAVITY: f32 = 9.81;
/// The Beaufort number a preset's curves end at.
const LAST_BEAUFORT: f32 = 12.0;
/// The least wind, in metres a second, so that a calm day still ripples.
const LEAST_WIND: f32 = 1.0;
/// Where the waves travel to: west, as the trade wind blows (assumed: the day states no wind's direction the water reads).
const TRAVEL: f32 = std::f32::consts::PI;
/// The waves of an octave: how far each turns from the wind, in degrees, and how its length stands to the octave's.
const SPREAD: [f32; 4] = [-31.0, -9.0, 13.0, 34.0];
const STRETCH: [f32; 4] = [1.0, 0.83, 1.19, 0.71];
/// How much shorter each octave's waves are than the one before.
const OCTAVE_STEP: f32 = 4.0;
/// How much the spectrum's peak is sharpened, and its width below and above the peak.
const PEAK_SHARPENING: f32 = 3.3;
const PEAK_WIDTH: [f32; 2] = [0.07, 0.09];
/// The fetch, in kilometres, where the day states none.
const FETCH_KM: f32 = 100.0;
/// How far each octave turns from the one before, in degrees, so that no two line up.
const OCTAVE_TURN: f32 = 17.0;

/// The documents of the deep ocean's three cascades, the first the longest.
const CASCADE_DOCUMENTS: [&str; 3] = [
    "data/water/oceanwavesimparams0.meta",
    "data/water/oceanwavesimparams1.meta",
    "data/water/oceanwavesimparams2.meta",
];
/// How far each cascade's spreading goes from even (nought) to the cosine squared (one).
const CASCADE_SPREADING: [f32; 3] = [0.9, 0.7, 0.9];
/// Where each cascade's band of numbers ends, in steps of two pi over its patch's side; each begins where the one before ends.
const BAND_TOP: [f32; 3] = [51.968, 64.0, 128.0];
/// The first cascade's waves lie north and south: the game gives its wind the angle nought, about north.
const FIRST_AXIS: f32 = std::f32::consts::FRAC_PI_2;
/// The steps a band's spectrum is summed over.
const BAND_STEPS: usize = 96;

type Read = Arc<OnceLock<Option<Arc<OceanPresets>>>>;
/// The deep ocean's three cascades.
pub type Cascades = [WaveParams; 3];

static HELD: Mutex<Vec<(PathBuf, Read)>> = Mutex::new(Vec::new());
static CASCADES_HELD: Slots<Cascades> = Mutex::new(Vec::new());

/// The ocean's presets of the install at `root`, read once on a worker and held: `None` until read, and where they do not read.
pub fn presets(root: &Path) -> Option<Arc<OceanPresets>> {
    held(&HELD, root, "the ocean's presets", read)
}

/// Whether the presets of the install at `root` were asked for and are still being read.
pub fn presets_reading(root: &Path) -> bool {
    reading(&HELD, root)
}

/// The deep ocean's three cascades of the install at `root`, read once on a worker and held: `None` until read, and where they do not read.
pub fn cascades(root: &Path) -> Option<Arc<Cascades>> {
    held(&CASCADES_HELD, root, "the ocean's cascades", read_cascades)
}

/// Whether the cascades of the install at `root` were asked for and are still being read.
pub fn cascades_reading(root: &Path) -> bool {
    reading(&CASCADES_HELD, root)
}

// ⚠ The documents are ciphered: the table names them, and only the deciphering reader opens them.
fn read_cascades(root: &Path, keys: &GtaViKeys) -> Result<Cascades, String> {
    let archive = root.join(ARCHIVE);
    let mut file = File::open(&archive).map_err(|e| e.to_string())?;
    let mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
    let mut one = |path: &str| -> Result<WaveParams, String> {
        let at = entry_offset(&archive, path, keys).ok_or_else(|| format!("no {path}"))?;
        let entry = (mount.archive.entries.iter())
            .find(|e| mount.absolute_start(e) == Some(at))
            .ok_or_else(|| format!("no entry at {at}"))?;
        let body =
            (mount.read_entry_decrypted(&mut file, entry, keys)).map_err(|e| e.to_string())?;
        WaveParams::parse(&String::from_utf8_lossy(&body)).map_err(|e| e.to_string())
    };
    Ok([
        one(CASCADE_DOCUMENTS[0])?,
        one(CASCADE_DOCUMENTS[1])?,
        one(CASCADE_DOCUMENTS[2])?,
    ])
}

fn read(root: &Path, keys: &GtaViKeys) -> Result<OceanPresets, String> {
    let archive = root.join(ARCHIVE);
    let at = entry_offset(&archive, ENTRY, keys).ok_or("the table names no ocean presets")?;
    let file = File::open(&archive).map_err(|e| e.to_string())?;
    let (_, text) = read_rpf9_body(&file, at, BODY_LIMIT).map_err(|e| e.to_string())?;
    OceanPresets::parse(&String::from_utf8_lossy(&text)).map_err(|e| e.to_string())
}

/// The sea's spectrum a metre of wave number at `number` for a wind of `wind` metres a second over `fetch` metres: JONSWAP's, its height squared a metre of number.
fn spectrum(number: f32, wind: f32, fetch: f32) -> f32 {
    let wind = wind.max(0.001);
    let reach = GRAVITY * fetch / (wind * wind);
    let alpha = 0.076 * reach.powf(-0.22);
    let peak = 22.0 * GRAVITY / wind * reach.powf(-1.0 / 3.0);
    let omega = (GRAVITY * number.max(1e-6)).sqrt();
    let width = if omega <= peak {
        PEAK_WIDTH[0]
    } else {
        PEAK_WIDTH[1]
    };
    let off = (omega / peak - 1.0) / width;
    let sharpened = PEAK_SHARPENING.powf((-0.5 * off * off).exp());
    // The spectrum by frequency, alpha g^2 / omega^5, carried to wave number by d omega / d k = g / (2 omega).
    alpha * GRAVITY.powi(3) / (2.0 * omega.powi(6))
        * (-1.25 * (peak / omega).powi(4)).exp()
        * sharpened
}

/// The number a metre at the peak of the spectrum for a wind of `wind` metres a second over `fetch` metres.
fn peak_number(wind: f32, fetch: f32) -> f32 {
    let wind = wind.max(0.001);
    let peak = 22.0 * GRAVITY / wind * (GRAVITY * fetch / (wind * wind)).powf(-1.0 / 3.0);
    peak * peak / GRAVITY
}

/// The height squared and the slope squared of the spectrum between the numbers `low` and `high` for a wind of `wind` over `fetch` metres.
fn band(low: f32, high: f32, wind: f32, fetch: f32) -> (f32, f32) {
    if high <= low || low <= 0.0 {
        return (0.0, 0.0);
    }
    let ratio = (high / low).powf(1.0 / BAND_STEPS as f32);
    (0..BAND_STEPS).fold((0.0, 0.0), |(m0, m2), i| {
        let a = low * ratio.powi(i as i32);
        let (k, dk) = (a * ratio.sqrt(), a * (ratio - 1.0));
        let s = spectrum(k, wind, fetch) * dk;
        (m0 + s, m2 + k * k * s)
    })
}

/// The waves of a sea whose three cascades are `cascades`, as [`waves`] gives them: each octave a cascade's band of numbers, its waves holding the band's height and slopes.
fn cascade_waves(
    cascades: &Cascades,
    preset: Option<&OceanPreset>,
    beaufort: f32,
    fetch: f32,
    seconds: f64,
) -> [[f32; 4]; WAVES] {
    let keys = cascades.each_ref().map(|c| c.at(beaufort));
    // ⚠ The game takes the wind of the first cascade for all three, and no wind is no sea.
    let wind = keys[0].wind_speed;
    let step = keys.map(|k| std::f32::consts::TAU / k.size.max(1e-3));
    let mut low = step[0];
    let bands: [(f32, f32); 3] = std::array::from_fn(|c| {
        let (from, to) = (low.max(step[c]), BAND_TOP[c] * step[c]);
        low = to;
        if wind > 0.0 {
            band(from, to, wind, fetch)
        } else {
            (0.0, 0.0)
        }
    });
    let along = (beaufort / LAST_BEAUFORT).clamp(0.0, 1.0);
    let each = SPREAD.len();
    // The four lengths about the band's own, scaled so that together they hold its slopes.
    let mean = (STRETCH.iter().map(|s| 1.0 / (s * s)).sum::<f32>() / each as f32).sqrt();
    // How much of a band each of the four ways holds: the game's spreading, blended from even to the cosine squared by the cascade's own measure.
    let spreading = |d: f32, spread: f32| (1.0 - d) + d * 2.0 * spread.to_radians().cos().powi(2);
    std::array::from_fn(|i| {
        let (octave, k) = (i / each, i % each);
        let (Some((spread, stretch)), Some(&(m0, m2))) =
            (SPREAD.get(k).zip(STRETCH.get(k)), bands.get(octave))
        else {
            return [0.0; 4];
        };
        if m0 <= 0.0 {
            return [0.0; 4];
        }
        let d = CASCADE_SPREADING.get(octave).copied().unwrap_or(1.0);
        let share = spreading(d, *spread) / SPREAD.iter().map(|s| spreading(d, *s)).sum::<f32>();
        let axis = if octave == 0 { FIRST_AXIS } else { TRAVEL };
        let angle = axis + (spread + OCTAVE_TURN * octave as f32).to_radians();
        let number = (m2 / m0).sqrt() / (stretch * mean);
        let strength = preset.map_or(1.0, |p| p.strength(octave, along));
        let amplitude = (2.0 * m0 * share).sqrt() * strength;
        let speed = f64::from((GRAVITY * number).sqrt());
        let start = f64::from((i as f32 * 2.399_963).rem_euclid(std::f32::consts::TAU));
        let phase = (start - speed * seconds).rem_euclid(std::f64::consts::TAU) as f32;
        [number * angle.cos(), number * angle.sin(), amplitude, phase]
    })
}

/// The waves of a sea at Beaufort `beaufort` over a fetch of `fetch_km` of the kind `preset` is, `seconds` into the day: each wave's number a metre along x and y, its height and its phase now. With the game's `cascades`, its three bands and its wind; without, one wind of the Beaufort number.
pub fn waves(
    cascades: Option<&Cascades>,
    preset: Option<&OceanPreset>,
    beaufort: f32,
    fetch_km: Option<f32>,
    seconds: f64,
) -> [[f32; 4]; WAVES] {
    let fetch = 1000.0 * fetch_km.filter(|f| *f > 0.0).unwrap_or(FETCH_KM);
    if let Some(cascades) = cascades {
        return cascade_waves(cascades, preset, beaufort, fetch, seconds);
    }
    let wind = (0.836 * beaufort.max(0.0).powf(1.5)).max(LEAST_WIND);
    let peak = peak_number(wind, fetch);
    let along = (beaufort / LAST_BEAUFORT).clamp(0.0, 1.0);
    let each = SPREAD.len();
    // Each octave stands for the numbers from half to twice its own, its four waves for a quarter of the half circle about the wind each.
    let across = std::f32::consts::PI / each as f32;
    std::array::from_fn(|i| {
        let (octave, k) = (i / each, i % each);
        let Some((spread, stretch)) = SPREAD.get(k).zip(STRETCH.get(k)) else {
            return [0.0; 4];
        };
        let own = peak * OCTAVE_STEP.powi(octave as i32);
        let number = own / stretch;
        let strength = preset.map_or(1.0, |p| p.strength(octave, along));
        let spreading = 2.0 / std::f32::consts::PI * spread.to_radians().cos().powi(2);
        let angle = TRAVEL + (spread + OCTAVE_TURN * octave as f32).to_radians();
        // A sine of height a holds a^2 / 2 of the sea's height squared.
        let share = spectrum(number, wind, fetch) * spreading * 1.5 * own * across;
        let amplitude = (2.0 * share).sqrt() * strength;
        let speed = f64::from((GRAVITY * number).sqrt());
        // A phase of its own at the day's start for each wave, and the phase now worked out in doubles.
        let start = f64::from((i as f32 * 2.399_963).rem_euclid(std::f32::consts::TAU));
        let phase = (start - speed * seconds).rem_euclid(std::f64::consts::TAU) as f32;
        [number * angle.cos(), number * angle.sin(), amplitude, phase]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spectrum_peaks_where_jonswap_puts_it_and_a_longer_fetch_lengthens_it() {
        let (wind, fetch) = (8.0, 100_000.0);
        let peak = peak_number(wind, fetch);
        let sampled = (1..4000).map(|i| peak * i as f32 / 1000.0);
        let best = sampled
            .max_by(|a, b| spectrum(*a, wind, fetch).total_cmp(&spectrum(*b, wind, fetch)))
            .unwrap_or(0.0);
        // The frequency spectrum peaks at the peak; carried to wave number, over a falling d omega / d k, a little short of it.
        assert!(best < peak && best > 0.9 * peak, "{best} against {peak}");
        assert!(
            peak_number(wind, 2.0 * fetch) < peak,
            "a longer fetch, longer waves"
        );
        assert!(
            peak_number(2.0 * wind, fetch) < peak,
            "a stronger wind, longer waves"
        );
        // The sea's height squared, summed over the numbers: JONSWAP's own, alpha g^2 / (5 omega_p^4) times about 1.5 for the sharpened peak.
        let step = peak / 200.0;
        let m0: f32 = (1..20_000)
            .map(|i| spectrum(step * i as f32, wind, fetch) * step)
            .sum();
        let reach = GRAVITY * fetch / (wind * wind);
        let omega = (GRAVITY * peak).sqrt();
        let plain = 0.076 * reach.powf(-0.22) * GRAVITY * GRAVITY / (5.0 * omega.powi(4));
        assert!(m0 > 1.2 * plain && m0 < 2.0 * plain, "{m0} against {plain}");
    }

    #[test]
    fn the_waves_follow_the_wind_the_preset_and_the_clock() {
        let calm = waves(None, None, 0.0, None, 0.0);
        let fresh = waves(None, None, 5.0, None, 0.0);
        let height = |w: &[[f32; 4]; WAVES]| w.iter().map(|w| w[2]).sum::<f32>();
        assert!(
            height(&fresh) > 5.0 * height(&calm),
            "a fresh breeze raises the sea"
        );
        let number = |w: [f32; 4]| w[0].hypot(w[1]);
        assert!(
            number(fresh[0]) < number(calm[0]),
            "and lengthens its waves"
        );
        assert!(
            number(fresh[8]) > 10.0 * number(fresh[0]),
            "the third octave's waves are sixteen times shorter"
        );
        assert!(
            fresh[8][2] < fresh[4][2] && fresh[4][2] < fresh[0][2],
            "shorter octaves are lower"
        );
        assert!(fresh[0][0] < 0.0, "the first wave travels west");
        let short = waves(None, None, 5.0, Some(1.0), 0.0);
        assert!(
            number(short[0]) > number(fresh[0]),
            "a short fetch, shorter waves"
        );
        let later = waves(None, None, 5.0, None, 3600.0 * 15.0);
        assert!(later.iter().zip(&fresh).all(|(a, b)| a[..3] == b[..3]));
        assert!(later
            .iter()
            .all(|w| (0.0..std::f32::consts::TAU).contains(&w[3])));
        assert_ne!(later[0][3], fresh[0][3], "the clock moves the phase");
        let still = OceanPreset {
            octaves: vec![vec![[0.0, 0.0], [1.0, 0.0]]],
            ..OceanPreset::default()
        };
        let stilled = waves(None, Some(&still), 5.0, None, 0.0);
        assert!(
            stilled[..4].iter().all(|w| w[2] == 0.0),
            "a preset's octave of no strength"
        );
        assert_eq!(stilled[4][2], fresh[4][2], "an octave it does not state");
    }

    #[test]
    fn the_cascades_hold_the_height_and_slopes_of_their_bands() {
        use or_io::gta6::water::waves::WaveKey;
        let cascade = |wind: f32, size: f32| WaveParams {
            keys: vec![
                WaveKey {
                    wind_speed: wind,
                    size
                };
                13
            ],
        };
        let sea = [cascade(4.5, 804.0), cascade(0.0, 110.0), cascade(0.0, 8.0)];
        let drawn = waves(Some(&sea), None, 3.0, Some(100.0), 0.0);
        let fetch = 100_000.0;
        // The bands meet: the first ends at 51.968 steps of its own, the second at 64, the third at 128.
        let step = [804.0_f32, 110.0, 8.0].map(|l| std::f32::consts::TAU / l);
        let edges = [step[0], 51.968 * step[0], 64.0 * step[1], 128.0 * step[2]];
        let (m0, m2) = (0..3)
            .map(|c| band(edges[c], edges[c + 1], 4.5, fetch))
            .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        let height: f32 = drawn.iter().map(|w| 0.5 * w[2] * w[2]).sum();
        let slope: f32 = drawn
            .iter()
            .map(|w| 0.5 * w[2] * w[2] * (w[0] * w[0] + w[1] * w[1]))
            .sum();
        assert!((height / m0 - 1.0).abs() < 0.05, "{height} against {m0}");
        assert!((slope / m2 - 1.0).abs() < 0.05, "{slope} against {m2}");
        assert!(
            drawn[1][1].abs() > drawn[1][0].abs(),
            "the first cascade runs north and south"
        );
        assert!(drawn[8][0].hypot(drawn[8][1]) > 10.0 * drawn[0][0].hypot(drawn[0][1]));
        let still = [cascade(0.0, 804.0), cascade(0.0, 110.0), cascade(0.0, 8.0)];
        assert!(
            waves(Some(&still), None, 3.0, None, 0.0)
                .iter()
                .all(|w| w[2] == 0.0),
            "no wind, no sea"
        );
    }
}
