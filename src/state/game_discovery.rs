use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use or_title::install::discovery::{discover_installs, DiscoveredInstall, DiscoverySource};

use crate::state::Game;

#[derive(Debug, Clone, Default)]
pub struct DiscoveryReport {
    pub installs: Vec<DiscoveredInstall>,
    pub icons: Vec<(Game, PathBuf)>,
    pub elapsed: Duration,
}

#[derive(Default)]
pub struct DiscoveryWorker {
    rx: Option<Receiver<DiscoveryReport>>,
}

impl DiscoveryWorker {
    pub fn is_running(&self) -> bool {
        self.rx.is_some()
    }

    pub fn spawn(known_paths: Vec<(Game, PathBuf)>) -> Self {
        let (tx, rx) = channel::<DiscoveryReport>();
        let spawn_result = thread::Builder::new()
            .name("game-discovery".into())
            .spawn(move || {
                let started = Instant::now();
                let mut installs = discover_installs();

                for (game, dir) in known_paths {
                    if installs.iter().any(|i| i.game == game) {
                        continue;
                    }
                    if let Some(install) =
                        DiscoveredInstall::probe(game, dir, DiscoverySource::UserConfigured)
                    {
                        installs.push(install);
                    }
                }

                let mut icons = Vec::new();
                // ⚠ A test build leaves the installs' executables alone: each test process would read
                // the owner's `GTA5.exe` and could be writing an icon into its scratch folder as it ends.
                for inst in installs.iter().filter(|_| !cfg!(test)) {
                    match extract_and_cache_icon(inst) {
                        Ok(path) => {
                            tracing::info!(
                                game = ?inst.game,
                                exe = %inst.exe.display(),
                                cache = %path.display(),
                                "Cached game icon",
                            );
                            icons.push((inst.game, path));
                        }
                        Err(e) => tracing::debug!(
                            game = ?inst.game,
                            exe = %inst.exe.display(),
                            error = %e,
                            "Icon extraction failed; the start page paints its fallback",
                        ),
                    }
                }

                let report = DiscoveryReport {
                    installs,
                    icons,
                    elapsed: started.elapsed(),
                };
                tracing::info!(
                    found = report.installs.len(),
                    icons = report.icons.len(),
                    elapsed_ms = report.elapsed.as_millis() as u64,
                    "Game discovery completed",
                );
                let _ = tx.send(report);
            });

        if let Err(e) = &spawn_result {
            tracing::warn!(error = %e, "Failed to spawn discovery thread");
        }
        Self {
            rx: spawn_result.ok().map(|_| rx),
        }
    }

    pub fn poll(&mut self) -> Option<DiscoveryReport> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(report) => {
                self.rx = None;
                Some(report)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }
}

/// Where extracted game icons are cached; a redirected configuration keeps its own.
pub fn icon_cache_dir() -> Option<PathBuf> {
    // ⚠ A self-check, `just shots` and the test binary redirect the configuration, and must not
    // write the owner's local data directory either.
    let config = crate::state::prefs::config_root();
    if config.is_some() && config != crate::state::prefs::real_config_root() {
        return config.map(|d| d.join("logos"));
    }
    dirs::data_local_dir().map(|d| d.join("openrage").join("logos"))
}

pub fn cached_icon_path(game: Game) -> Option<PathBuf> {
    icon_cache_dir().map(|d| d.join(format!("{}.png", game.asset_slug())))
}

pub fn extract_and_cache_icon(install: &DiscoveredInstall) -> std::io::Result<PathBuf> {
    let Some(out) = cached_icon_path(install.game) else {
        return Err(std::io::Error::other("no local data dir available"));
    };
    if let Ok(meta) = std::fs::metadata(&out) {
        if meta.is_file() && meta.len() >= 64 {
            return Ok(out);
        }
        let _ = std::fs::remove_file(&out);
    }
    let img = read_largest_icon(&install.exe)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    img.save(&out).map_err(std::io::Error::other)?;
    tracing::debug!(
        exe = %install.exe.display(),
        out = %out.display(),
        size = format!("{}x{}", img.width(), img.height()),
        "Cached game icon",
    );
    Ok(out)
}

fn read_largest_icon(exe: &Path) -> std::io::Result<image::DynamicImage> {
    use pelite::resources::{Entry, Name};
    use pelite::PeFile;

    let bytes = std::fs::read(exe)?;
    let pe = PeFile::from_bytes(&bytes).map_err(invalid_data)?;
    let resources = pe.resources().map_err(invalid_data)?;
    let root = resources.root().map_err(invalid_data)?;

    let group_dir = root
        .get_dir(Name::Id(14))
        .map_err(|_| not_found("no RT_GROUP_ICON in exe"))?;
    let icon_dir = root
        .get_dir(Name::Id(3))
        .map_err(|_| not_found("no RT_ICON in exe"))?;

    let mut entries: Vec<IcoEntryRaw> = Vec::new();
    for group_entry in group_dir.entries() {
        let lang_dir = match group_entry.entry().map_err(invalid_data)? {
            Entry::Directory(d) => d,
            Entry::DataEntry(_) => continue,
        };
        let Some(lang_entry) = lang_dir.entries().next() else {
            continue;
        };
        let group_data = match lang_entry.entry().map_err(invalid_data)? {
            Entry::DataEntry(d) => d.bytes().map_err(invalid_data)?,
            Entry::Directory(_) => continue,
        };
        let group = match parse_group_icon_dir(group_data) {
            Ok(g) => g,
            Err(_) => continue,
        };
        for ge in &group.entries {
            let Ok(data) = lookup_icon_bytes(&icon_dir, ge.id) else {
                continue;
            };
            entries.push(IcoEntryRaw {
                width: ge.width,
                height: ge.height,
                colors: ge.colors,
                planes: ge.planes,
                bits_per_pixel: ge.bits_per_pixel,
                data: data.to_vec(),
            });
        }
    }

    if entries.is_empty() {
        return Err(not_found("no usable icon entries in exe"));
    }

    entries.sort_by_key(|e| {
        let w = if e.width == 0 { 256u32 } else { e.width as u32 };
        let h = if e.height == 0 {
            256u32
        } else {
            e.height as u32
        };
        std::cmp::Reverse((w * h, e.bits_per_pixel))
    });

    let mut last_err: Option<image::ImageError> = None;
    for entry in &entries {
        match decode_icon_entry(entry) {
            Ok(img) => return Ok(img),
            Err(e) => last_err = Some(e),
        }
    }
    Err(invalid_data(
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown decode error".into()),
    ))
}

fn decode_icon_entry(entry: &IcoEntryRaw) -> Result<image::DynamicImage, image::ImageError> {
    const PNG_SIG: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if entry.data.starts_with(PNG_SIG) {
        return image::load_from_memory_with_format(&entry.data, image::ImageFormat::Png);
    }
    let ico_bytes = build_ico_file(std::slice::from_ref(entry));
    image::load_from_memory_with_format(&ico_bytes, image::ImageFormat::Ico)
}

fn invalid_data<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
}

fn not_found(msg: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, msg)
}

#[derive(Debug)]
struct GroupIconDir {
    entries: Vec<GroupIconEntry>,
}

#[derive(Debug, Clone)]
struct GroupIconEntry {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits_per_pixel: u16,
    id: u16,
}

struct IcoEntryRaw {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits_per_pixel: u16,
    data: Vec<u8>,
}

fn parse_group_icon_dir(bytes: &[u8]) -> std::io::Result<GroupIconDir> {
    if bytes.len() < 6 {
        return Err(invalid_data("group icon directory too small"));
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let entries_start = 6;
    let entries_end = entries_start + count * 14;
    if bytes.len() < entries_end {
        return Err(invalid_data("group icon directory entries truncated"));
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = entries_start + i * 14;
        entries.push(GroupIconEntry {
            width: bytes[off],
            height: bytes[off + 1],
            colors: bytes[off + 2],
            planes: u16::from_le_bytes([bytes[off + 4], bytes[off + 5]]),
            bits_per_pixel: u16::from_le_bytes([bytes[off + 6], bytes[off + 7]]),
            id: u16::from_le_bytes([bytes[off + 12], bytes[off + 13]]),
        });
    }
    Ok(GroupIconDir { entries })
}

fn lookup_icon_bytes<'a>(
    icon_dir: &pelite::resources::Directory<'a>,
    id: u16,
) -> std::io::Result<&'a [u8]> {
    use pelite::resources::{Entry, Name};

    let name_dir = icon_dir
        .get_dir(Name::Id(id as u32))
        .map_err(|_| not_found("RT_ICON id missing"))?;
    let lang_entry = name_dir
        .entries()
        .next()
        .ok_or_else(|| not_found("RT_ICON name dir empty"))?;
    let data_entry = match lang_entry.entry().map_err(invalid_data)? {
        Entry::DataEntry(d) => d,
        Entry::Directory(_) => {
            return Err(invalid_data("RT_ICON language entry is a directory"));
        }
    };
    data_entry.bytes().map_err(invalid_data)
}

fn build_ico_file(entries: &[IcoEntryRaw]) -> Vec<u8> {
    let header_len = 6 + entries.len() * 16;
    let total_data_len: usize = entries.iter().map(|e| e.data.len()).sum();
    let mut out = Vec::with_capacity(header_len + total_data_len);

    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut data_offset = header_len as u32;
    for e in entries {
        out.push(e.width);
        out.push(e.height);
        out.push(e.colors);
        out.push(0);
        out.extend_from_slice(&e.planes.to_le_bytes());
        out.extend_from_slice(&e.bits_per_pixel.to_le_bytes());
        out.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data_offset.to_le_bytes());
        data_offset += e.data.len() as u32;
    }
    for e in entries {
        out.extend_from_slice(&e.data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redirected_configuration_keeps_the_icon_cache_beside_it() {
        let config = crate::state::prefs::config_root().expect("the test binary's scratch");
        assert_ne!(
            Some(&config),
            crate::state::prefs::real_config_root().as_ref()
        );
        let cache = icon_cache_dir().expect("an icon cache");
        assert!(
            cache.starts_with(&config),
            "{} is outside the redirected configuration {}",
            cache.display(),
            config.display()
        );
    }
}
