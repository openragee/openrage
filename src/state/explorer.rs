//! How the archive tree is ordered and which places the user keeps: the half
//! of the explorer that outlives a session.

use std::cmp::Ordering;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::document::RpfNodeId;
use super::rpf_node::{RpfNodeBacking, RpfSessionNode};
use super::rpf_session::RpfEditSession;
use or_io::container::rpf::Rpf9Kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Type,
    /// The archive's own table order.
    Stored,
    StoredSize,
    Compression,
    Encryption,
    Edition,
    Version,
}

impl SortKey {
    pub const ALL: [SortKey; 9] = [
        Self::Name,
        Self::Size,
        Self::Type,
        Self::Stored,
        Self::StoredSize,
        Self::Compression,
        Self::Encryption,
        Self::Edition,
        Self::Version,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => tr!("Name"),
            Self::Size => tr!("Size"),
            Self::Type => tr!("Type"),
            Self::Stored => tr!("Archive order"),
            Self::StoredSize => tr!("Stored size"),
            Self::Compression => tr!("Compression"),
            Self::Encryption => tr!("Encryption"),
            Self::Edition => tr!("Edition"),
            Self::Version => tr!("Version"),
        }
    }

    /// Whether rows sort by one of the Details columns past the name, type and size.
    fn reads_a_column(self) -> bool {
        matches!(
            self,
            Self::StoredSize | Self::Compression | Self::Encryption | Self::Edition | Self::Version
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SortOrder {
    #[serde(default)]
    pub key: SortKey,
    #[serde(default)]
    pub descending: bool,
}

/// What a comparison needs to know about one row.
#[derive(Debug, Clone)]
pub struct SortRow<'a> {
    pub name: &'a str,
    pub size: Option<u64>,
    pub is_dir: bool,
    /// The Details column a [`SortKey`] past the name, type and size reads.
    pub column: Option<ColumnValue>,
}

/// One Details cell, as a sort compares it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColumnValue {
    Number(u64),
    Text(&'static str),
}

/// The order two rows appear in.
///
/// ⚠ **Folders come first in both directions**, which is what Explorer and
/// OpenIV do; reversing a sort that put them last would scatter them through a
/// long file list. Ties fall back to the name ascending, whatever the
/// direction, so equal sizes read alphabetically rather than in whatever order
/// the archive stored them.
pub fn compare(a: &SortRow, b: &SortRow, order: SortOrder) -> Ordering {
    if order.key == SortKey::Stored {
        return Ordering::Equal;
    }
    match (a.is_dir, b.is_dir) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    // ⚠ A row with nothing in the column goes last in both directions, as a
    // folder goes first: reversing would otherwise lead with blanks.
    match (&a.column, &b.column) {
        (Some(_), None) if order.key.reads_a_column() => return Ordering::Less,
        (None, Some(_)) if order.key.reads_a_column() => return Ordering::Greater,
        _ => {}
    }
    let primary = match order.key {
        SortKey::Name | SortKey::Stored => natural(a.name, b.name),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Type => extension(a.name).cmp(&extension(b.name)),
        _ => a.column.cmp(&b.column),
    };
    let primary = if order.descending {
        primary.reverse()
    } else {
        primary
    };
    primary
        .then_with(|| natural(a.name, b.name))
        .then_with(|| a.name.cmp(b.name))
}

/// Case-insensitive, with runs of digits compared as numbers, so `lod2` sorts
/// before `lod10` the way a person reads them.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (da, ra) = split_digits(a);
                let (db, rb) = split_digits(b);
                let (ta, tb) = (trim_zeros(da), trim_zeros(db));
                let by_value = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if by_value != Ordering::Equal {
                    return by_value;
                }
                (a, b) = (ra, rb);
            }
            (Some(x), Some(y)) => {
                let by_char = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if by_char != Ordering::Equal {
                    return by_char;
                }
                (a, b) = (&a[1..], &b[1..]);
            }
        }
    }
}

fn split_digits(s: &[u8]) -> (&[u8], &[u8]) {
    let end = s
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(s.len());
    s.split_at(end)
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let start = digits
        .iter()
        .position(|&c| c != b'0')
        .unwrap_or(digits.len());
    &digits[start..]
}

fn extension(name: &str) -> String {
    super::extract_job::extension_of(name)
}

/// A folder's children in the order `order` puts them.
pub fn sorted_children(
    session: &RpfEditSession,
    parent: RpfNodeId,
    order: SortOrder,
) -> Vec<RpfNodeId> {
    let children = session.children_of(parent);
    if order.key == SortKey::Stored {
        return children;
    }
    let mut rows: Vec<(RpfNodeId, SortRow)> = children
        .into_iter()
        .filter_map(|id| {
            let node = session.node(id)?;
            let size = match order.key {
                SortKey::Size if !node.is_dir() => session.size_for(id),
                _ => None,
            };
            let column = order
                .key
                .reads_a_column()
                .then(|| super::files_view::column_value(session, id, order.key))
                .flatten();
            Some((
                id,
                SortRow {
                    name: &node.name,
                    size,
                    is_dir: node.is_dir(),
                    column,
                },
            ))
        })
        .collect();
    rows.sort_by(|a, b| compare(&a.1, &b.1, order));
    rows.into_iter().map(|r| r.0).collect()
}

/// How the Files view in the centre lays a folder out; Files on the left is always the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ViewMode {
    /// A row per entry with its columns.
    #[default]
    Details,
    /// Tiles, each with a picture of its entry.
    Icons,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Details => tr!("Details"),
            Self::Icons => tr!("Icons"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GroupKey {
    #[default]
    None,
    Type,
    Size,
}

impl GroupKey {
    pub const ALL: [GroupKey; 3] = [Self::None, Self::Type, Self::Size];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => tr!("No grouping"),
            Self::Type => tr!("Type"),
            Self::Size => tr!("Size"),
        }
    }
}

/// Explorer's own size groups, so a user reads the same names in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeBucket {
    Empty,
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
    Gigantic,
    Unknown,
}

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

impl SizeBucket {
    pub fn of(size: Option<u64>) -> Self {
        match size {
            None => Self::Unknown,
            Some(0) => Self::Empty,
            Some(n) if n <= 16 * KIB => Self::Tiny,
            Some(n) if n <= MIB => Self::Small,
            Some(n) if n <= 128 * MIB => Self::Medium,
            Some(n) if n <= GIB => Self::Large,
            Some(n) if n <= 4 * GIB => Self::Huge,
            Some(_) => Self::Gigantic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => tr!("Empty (0 KB)"),
            Self::Tiny => tr!("Tiny (0 – 16 KB)"),
            Self::Small => tr!("Small (16 KB – 1 MB)"),
            Self::Medium => tr!("Medium (1 – 128 MB)"),
            Self::Large => tr!("Large (128 MB – 1 GB)"),
            Self::Huge => tr!("Huge (1 – 4 GB)"),
            Self::Gigantic => tr!("Gigantic (> 4 GB)"),
            Self::Unknown => tr!("Size not known"),
        }
    }
}

/// One heading in a grouped folder, and its rows in the sort order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub label: String,
    pub rows: Vec<RpfNodeId>,
}

/// A folder's children cut into groups.
///
/// ⚠ **Grouping never reorders what the sort decided inside a group**, and
/// folders are a group of their own that leads, as they lead an ungrouped
/// list. Type groups read in extension order and size groups smallest first,
/// whatever direction the sort runs, because a group's heading is a place to
/// find and not a value being sorted.
pub fn grouped_children(
    session: &RpfEditSession,
    parent: RpfNodeId,
    order: SortOrder,
    group: GroupKey,
) -> Vec<Group> {
    let sorted = sorted_children(session, parent, order);
    if group == GroupKey::None {
        return vec![Group {
            label: String::new(),
            rows: sorted,
        }];
    }
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Heading {
        Folders,
        Type(String),
        Size(SizeBucket),
    }
    let mut groups: Vec<(Heading, Vec<RpfNodeId>)> = Vec::new();
    for id in sorted {
        let Some(node) = session.node(id) else {
            continue;
        };
        let heading = if node.is_dir() {
            Heading::Folders
        } else if group == GroupKey::Type {
            Heading::Type(extension(&node.name))
        } else {
            Heading::Size(SizeBucket::of(session.size_for(id)))
        };
        match groups.iter_mut().find(|(h, _)| *h == heading) {
            Some((_, rows)) => rows.push(id),
            None => groups.push((heading, vec![id])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
        .into_iter()
        .map(|(heading, rows)| Group {
            label: match heading {
                Heading::Folders => tr!("Folders").to_string(),
                Heading::Type(ext) if ext.is_empty() => tr!("No extension").to_string(),
                Heading::Type(ext) => format!(".{ext}"),
                Heading::Size(bucket) => bucket.label().to_string(),
            },
            rows,
        })
        .collect()
}

/// The filter typed into the tree, as a glob the archive-wide search reads.
///
/// ⚠ A substring filter is a glob with stars round it and no separator:
/// `ydr` is `**/*ydr*`. Text that is already a glob is taken as meant.
pub fn escalated_glob(filter: &str) -> String {
    let filter = filter.trim();
    if filter.contains(['*', '?', '/']) {
        return filter.to_string();
    }
    format!("**/*{filter}*")
}

/// What a GTA VI file row is, as its archive's table names it; `None` for any other row.
pub fn gta6_kind(session: &RpfEditSession, node: &RpfSessionNode) -> Option<Rpf9Kind> {
    let RpfNodeBacking::ArchiveFile {
        archive_id,
        entry_index,
        ..
    } = &node.backing
    else {
        return None;
    };
    session
        .archive(*archive_id)?
        .archive
        .rpf9_kind(*entry_index)
}

/// Whether a file row holds the filter's lower-case `needle`: in its name, its hash as `0x` hex, or its kind's words or GTA V extension.
pub fn row_matches(name: &str, kind: Option<Rpf9Kind>, needle: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let by_hash = (needle.strip_prefix("0x"))
        .is_some_and(|hex| name.strip_prefix("hash_").is_some_and(|h| h.contains(hex)));
    by_hash
        || name.contains(needle)
        || kind.is_some_and(|kind| {
            kind.label.to_ascii_lowercase().contains(needle)
                || (kind.extension).is_some_and(|ext| format!(".{ext}").contains(needle))
        })
}

/// How many loaded files a substring filter admits, out of how many there are.
///
/// ⚠ **Loaded, not in the archive.** A nested archive nobody has expanded has
/// no nodes yet, so the denominator grows as the user opens things; the
/// readout says so rather than presenting a partial count as the archive's.
pub fn filter_counts(session: &RpfEditSession, filter: &str) -> (usize, usize) {
    let needle = filter.to_ascii_lowercase();
    let (mut shown, mut total) = (0, 0);
    for id in session.all_nodes() {
        let Some(node) = session.node(id) else {
            continue;
        };
        if node.is_dir() {
            continue;
        }
        total += 1;
        if row_matches(&node.name, gta6_kind(session, node), &needle) {
            shown += 1;
        }
    }
    (shown, total)
}

/// A location worth coming back to: a file, folder or install, and a path
/// inside it.
///
/// ⚠ **The source is text, not a `PathBuf`.** `serde_json` refuses to write a
/// path that is not valid Unicode, and it refuses the *whole* document — so a
/// favourite that could not be written would stop every preference saving.
/// [`Place::new`] refuses such a path instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub source: String,
    /// `/`-separated, relative to the source; empty for the source itself.
    #[serde(default)]
    pub inner: String,
}

impl Place {
    pub fn new(source: &Path, inner: &str) -> Option<Self> {
        Some(Self {
            source: source.to_str()?.to_string(),
            inner: inner.trim_matches('/').to_string(),
        })
    }

    pub fn label(&self) -> String {
        let source = Path::new(&self.source)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.source);
        if self.inner.is_empty() {
            source.to_string()
        } else {
            format!("{source}/{}", self.inner)
        }
    }
}

const MAX_FAVOURITES: usize = 64;

/// The explorer's slice of `prefs.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExplorerPrefs {
    pub sort: SortOrder,
    pub favourites: Vec<Place>,
    pub view: ViewMode,
    pub group: GroupKey,
}

impl ExplorerPrefs {
    pub fn is_favourite(&self, place: &Place) -> bool {
        self.favourites.contains(place)
    }

    /// Add `place`, or take it out if it is already there. Returns whether it
    /// is a favourite afterwards.
    pub fn toggle_favourite(&mut self, place: Place) -> bool {
        if let Some(at) = self.favourites.iter().position(|p| p == &place) {
            self.favourites.remove(at);
            return false;
        }
        if self.favourites.len() >= MAX_FAVOURITES {
            return false;
        }
        self.favourites.push(place);
        true
    }

    pub fn move_favourite(&mut self, from: usize, to: usize) {
        if from < self.favourites.len() && to < self.favourites.len() {
            let place = self.favourites.remove(from);
            self.favourites.insert(to, place);
        }
    }
}

impl super::AppState {
    pub fn set_explorer_sort(&mut self, order: SortOrder) {
        if self.prefs.explorer.sort != order {
            self.prefs.explorer.sort = order;
            super::save_prefs(&self.prefs);
        }
    }

    /// Add a place to the favourites or take it out, and say which.
    pub fn toggle_favourite(&mut self, place: Place) {
        let label = place.label();
        let was = self.prefs.explorer.is_favourite(&place);
        let now = self.prefs.explorer.toggle_favourite(place);
        if was == now {
            self.show_warning_toast(tr!(
                "Favourites are full ({max_favourites}); remove one to add {label}",
                max_favourites = MAX_FAVOURITES,
                label = label
            ));
            return;
        }
        super::save_prefs(&self.prefs);
        if now {
            self.show_toast(tr!("Added {label} to favourites", label = label));
        } else {
            self.show_toast(tr!("Removed {label} from favourites", label = label));
        }
    }

    pub fn move_favourite(&mut self, from: usize, to: usize) {
        self.prefs.explorer.move_favourite(from, to);
        super::save_prefs(&self.prefs);
    }

    pub fn set_explorer_view(&mut self, view: ViewMode) {
        if self.prefs.explorer.view != view {
            self.prefs.explorer.view = view;
            super::save_prefs(&self.prefs);
        }
    }

    pub fn set_explorer_group(&mut self, group: GroupKey) {
        if self.prefs.explorer.group != group {
            self.prefs.explorer.group = group;
            super::save_prefs(&self.prefs);
        }
    }
}

/// ⚠ **Every part of this is read on its own, so an unreadable part costs only
/// itself.** A field `serde` cannot parse fails the whole `Prefs` document,
/// which is every game path and shortcut the user has; here a bad sort order
/// is the default order and a bad favourite is one fewer favourite.
pub fn readable<'de, D>(deserializer: D) -> Result<ExplorerPrefs, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(read_explorer_prefs(&value))
}

fn read_explorer_prefs(value: &serde_json::Value) -> ExplorerPrefs {
    let sort = value
        .get("sort")
        .and_then(|v| serde_json::from_value::<SortOrder>(v.clone()).ok())
        .unwrap_or_default();
    let mut favourites: Vec<Place> = Vec::new();
    let mut dropped = 0usize;
    if let Some(list) = value.get("favourites").and_then(|v| v.as_array()) {
        for entry in list {
            match serde_json::from_value::<Place>(entry.clone()) {
                Ok(place) if !favourites.contains(&place) => favourites.push(place),
                Ok(_) => {}
                Err(_) => dropped += 1,
            }
        }
    }
    if dropped > 0 {
        tracing::warn!(dropped, "unreadable favourites were skipped");
    }
    favourites.truncate(MAX_FAVOURITES);
    let view = value
        .get("view")
        .and_then(|v| serde_json::from_value::<ViewMode>(v.clone()).ok())
        .unwrap_or_default();
    let group = value
        .get("group")
        .and_then(|v| serde_json::from_value::<GroupKey>(v.clone()).ok())
        .unwrap_or_default();
    ExplorerPrefs {
        sort,
        favourites,
        view,
        group,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file<'a>(name: &'a str, size: u64) -> SortRow<'a> {
        SortRow {
            name,
            size: Some(size),
            is_dir: false,
            column: None,
        }
    }

    fn dir(name: &str) -> SortRow<'_> {
        SortRow {
            name,
            size: None,
            is_dir: true,
            column: None,
        }
    }

    fn names<'a>(mut rows: Vec<SortRow<'a>>, order: SortOrder) -> Vec<&'a str> {
        rows.sort_by(|a, b| compare(a, b, order));
        rows.into_iter().map(|r| r.name).collect()
    }

    fn rows() -> Vec<SortRow<'static>> {
        vec![
            file("prop_lod10.ydr", 300),
            dir("textures"),
            file("Prop_lod2.ydr", 900),
            file("a.ytd", 300),
            dir("Models"),
            file("b.ybn", 50),
        ]
    }

    #[test]
    fn by_name_folders_lead_and_digits_read_as_numbers_whatever_the_case() {
        let order = SortOrder::default();
        assert_eq!(
            names(rows(), order),
            [
                "Models",
                "textures",
                "a.ytd",
                "b.ybn",
                "Prop_lod2.ydr",
                "prop_lod10.ydr"
            ]
        );
    }

    #[test]
    fn descending_reverses_the_files_and_still_leads_with_folders() {
        let order = SortOrder {
            key: SortKey::Name,
            descending: true,
        };
        assert_eq!(
            names(rows(), order),
            [
                "textures",
                "Models",
                "prop_lod10.ydr",
                "Prop_lod2.ydr",
                "b.ybn",
                "a.ytd"
            ]
        );
    }

    #[test]
    fn by_size_ties_read_alphabetically_in_both_directions() {
        let up = SortOrder {
            key: SortKey::Size,
            descending: false,
        };
        assert_eq!(
            names(rows(), up),
            [
                "Models",
                "textures",
                "b.ybn",
                "a.ytd",
                "prop_lod10.ydr",
                "Prop_lod2.ydr"
            ]
        );
        let down = SortOrder {
            key: SortKey::Size,
            descending: true,
        };
        assert_eq!(
            names(rows(), down),
            [
                "Models",
                "textures",
                "Prop_lod2.ydr",
                "a.ytd",
                "prop_lod10.ydr",
                "b.ybn"
            ]
        );
    }

    #[test]
    fn by_type_groups_extensions_and_names_within_them() {
        let order = SortOrder {
            key: SortKey::Type,
            descending: false,
        };
        assert_eq!(
            names(rows(), order),
            [
                "Models",
                "textures",
                "b.ybn",
                "Prop_lod2.ydr",
                "prop_lod10.ydr",
                "a.ytd"
            ]
        );
    }

    #[test]
    fn archive_order_is_the_order_the_rows_arrived_in() {
        let order = SortOrder {
            key: SortKey::Stored,
            descending: false,
        };
        let arrived: Vec<&str> = rows().into_iter().map(|r| r.name).collect();
        assert_eq!(names(rows(), order), arrived);
    }

    #[test]
    fn natural_order_is_total_on_leading_zeros() {
        assert_eq!(natural("lod02", "lod2"), Ordering::Equal);
        assert_eq!(natural("lod009", "lod10"), Ordering::Less);
        assert_eq!(natural("x", "x1"), Ordering::Less);
    }

    #[test]
    fn the_session_sort_applies_the_comparator_to_real_nodes() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        session
            .add_file_copy(root, "lod10.ydr", vec![0; 4])
            .unwrap();
        session.add_folder(root, "zeta").unwrap();
        session
            .add_file_copy(root, "lod2.ydr", vec![0; 40])
            .unwrap();

        let name_of = |ids: Vec<RpfNodeId>| -> Vec<String> {
            ids.into_iter()
                .map(|id| session.node(id).unwrap().name.clone())
                .collect()
        };
        assert_eq!(
            name_of(sorted_children(&session, root, SortOrder::default())),
            ["zeta", "lod2.ydr", "lod10.ydr"]
        );
        let by_size_down = SortOrder {
            key: SortKey::Size,
            descending: true,
        };
        assert_eq!(
            name_of(sorted_children(&session, root, by_size_down)),
            ["zeta", "lod2.ydr", "lod10.ydr"]
        );
        let by_size_up = SortOrder {
            key: SortKey::Size,
            descending: false,
        };
        assert_eq!(
            name_of(sorted_children(&session, root, by_size_up)),
            ["zeta", "lod10.ydr", "lod2.ydr"]
        );
    }

    #[test]
    fn the_filter_readout_counts_files_and_not_folders() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let models = session.add_folder(root, "models").unwrap();
        session.add_file_copy(models, "car.ydr", vec![1]).unwrap();
        session.add_file_copy(models, "car.ytd", vec![1]).unwrap();
        session.add_file_copy(root, "boat.ydr", vec![1]).unwrap();
        assert_eq!(filter_counts(&session, "YDR"), (2, 3));
        assert_eq!(filter_counts(&session, "models"), (0, 3));
    }

    #[test]
    fn a_favourite_toggles_and_keeps_its_place_in_the_list() {
        let mut prefs = ExplorerPrefs::default();
        let a = Place::new(Path::new("C:/g/x64a.rpf"), "/levels/gta5/").unwrap();
        let b = Place::new(Path::new("C:/g/x64b.rpf"), "").unwrap();
        assert_eq!(a.inner, "levels/gta5");
        assert!(prefs.toggle_favourite(a.clone()));
        assert!(prefs.toggle_favourite(b.clone()));
        prefs.move_favourite(1, 0);
        assert_eq!(prefs.favourites, vec![b.clone(), a.clone()]);
        assert!(!prefs.toggle_favourite(b.clone()));
        assert_eq!(prefs.favourites, vec![a]);
    }

    #[test]
    fn size_buckets_break_where_explorer_breaks_them() {
        let cases = [
            (Some(0), SizeBucket::Empty),
            (Some(1), SizeBucket::Tiny),
            (Some(16 * KIB), SizeBucket::Tiny),
            (Some(16 * KIB + 1), SizeBucket::Small),
            (Some(MIB), SizeBucket::Small),
            (Some(MIB + 1), SizeBucket::Medium),
            (Some(128 * MIB), SizeBucket::Medium),
            (Some(128 * MIB + 1), SizeBucket::Large),
            (Some(GIB), SizeBucket::Large),
            (Some(GIB + 1), SizeBucket::Huge),
            (Some(4 * GIB), SizeBucket::Huge),
            (Some(4 * GIB + 1), SizeBucket::Gigantic),
            (None, SizeBucket::Unknown),
        ];
        for (size, bucket) in cases {
            assert_eq!(SizeBucket::of(size), bucket, "{size:?}");
        }
    }

    fn grouped_session() -> (RpfEditSession, RpfNodeId) {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        session
            .add_file_copy(root, "big.ydr", vec![0; 20 * 1024])
            .unwrap();
        session.add_file_copy(root, "a.ytd", vec![0; 10]).unwrap();
        session.add_folder(root, "props").unwrap();
        session
            .add_file_copy(root, "empty.ydr", Vec::new())
            .unwrap();
        session.add_file_copy(root, "b.ytd", vec![0; 30]).unwrap();
        (session, root)
    }

    fn labelled(session: &RpfEditSession, groups: Vec<Group>) -> Vec<(String, Vec<String>)> {
        groups
            .into_iter()
            .map(|g| {
                (
                    g.label,
                    g.rows
                        .into_iter()
                        .map(|id| session.node(id).unwrap().name.clone())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn grouping_by_size_names_the_buckets_and_keeps_the_sort_inside_them() {
        let (session, root) = grouped_session();
        let descending = SortOrder {
            key: SortKey::Name,
            descending: true,
        };
        let groups = grouped_children(&session, root, descending, GroupKey::Size);
        assert_eq!(
            labelled(&session, groups),
            vec![
                ("Folders".to_string(), vec!["props".to_string()]),
                ("Empty (0 KB)".to_string(), vec!["empty.ydr".to_string()]),
                (
                    "Tiny (0 – 16 KB)".to_string(),
                    vec!["b.ytd".to_string(), "a.ytd".to_string()]
                ),
                (
                    "Small (16 KB – 1 MB)".to_string(),
                    vec!["big.ydr".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn grouping_by_type_heads_each_extension_and_ungrouped_is_one_group() {
        let (session, root) = grouped_session();
        let order = SortOrder::default();
        assert_eq!(
            labelled(
                &session,
                grouped_children(&session, root, order, GroupKey::Type)
            ),
            vec![
                ("Folders".to_string(), vec!["props".to_string()]),
                (
                    ".ydr".to_string(),
                    vec!["big.ydr".to_string(), "empty.ydr".to_string()]
                ),
                (
                    ".ytd".to_string(),
                    vec!["a.ytd".to_string(), "b.ytd".to_string()]
                ),
            ]
        );
        let flat = grouped_children(&session, root, order, GroupKey::None);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].rows, sorted_children(&session, root, order));
    }

    #[test]
    fn a_filter_escalates_to_the_glob_that_finds_it_anywhere() {
        assert_eq!(escalated_glob(" ydr "), "**/*ydr*");
        assert_eq!(escalated_glob("**/*.ytd"), "**/*.ytd");
        assert_eq!(escalated_glob("x64a.rpf/levels"), "x64a.rpf/levels");
    }

    #[test]
    fn an_unreadable_view_or_group_costs_only_itself() {
        let value = serde_json::json!({
            "sort": { "key": "Size", "descending": true },
            "view": "Carousel",
            "group": "Size",
        });
        let prefs = read_explorer_prefs(&value);
        assert_eq!(prefs.view, ViewMode::Details);
        assert_eq!(prefs.group, GroupKey::Size);
        assert_eq!(prefs.sort.key, SortKey::Size);

        let written = serde_json::to_value(ExplorerPrefs {
            view: ViewMode::Icons,
            group: GroupKey::Type,
            ..Default::default()
        })
        .unwrap();
        let read = read_explorer_prefs(&written);
        assert_eq!((read.view, read.group), (ViewMode::Icons, GroupKey::Type));
    }

    #[test]
    fn one_unreadable_favourite_costs_only_itself() {
        let value = serde_json::json!({
            "sort": { "key": "NoSuchKey", "descending": true },
            "favourites": [
                { "source": "C:/g/x64a.rpf", "inner": "levels" },
                { "source": 17 },
                "not even an object",
                { "source": "C:/g/x64a.rpf", "inner": "levels" },
                { "source": "C:/g/update.rpf" }
            ]
        });
        let prefs = read_explorer_prefs(&value);
        assert_eq!(prefs.sort, SortOrder::default());
        assert_eq!(
            prefs.favourites,
            vec![
                Place {
                    source: "C:/g/x64a.rpf".into(),
                    inner: "levels".into()
                },
                Place {
                    source: "C:/g/update.rpf".into(),
                    inner: String::new()
                },
            ]
        );
    }
}
