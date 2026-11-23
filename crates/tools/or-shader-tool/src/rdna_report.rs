//! What a device check of GTA VI's native programs found, as a file one machine writes and many machines' files merge into.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The report's layout, which a merge refuses to mix.
pub const VERSION: u32 = 1;

/// One line a worker process writes: what it is about to do, and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Line {
    /// Written before a step that can end the process; `adapter` is `None` while the translator runs.
    Begin {
        unit: usize,
        block: usize,
        adapter: Option<usize>,
    },
    /// The unit reached no device.
    Front {
        unit: usize,
        block: usize,
        why: String,
    },
    Device {
        unit: usize,
        block: usize,
        adapter: usize,
        outcome: Outcome,
        /// The milliseconds the device took over the shader modules and the pipeline.
        ms: f64,
    },
    /// Something the check did to the unit before any device saw it.
    Note {
        unit: usize,
        block: usize,
        what: String,
    },
    /// The worker reached the end of its range.
    Done,
}

/// What a device made of one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Created,
    /// The device's own words, on one line.
    Rejected(String),
    /// The device feature the unit needs and the device lacks.
    Skipped(String),
    /// The process ended inside the driver, with this exit status.
    Crashed(String),
    TimedOut,
}

/// One adapter as wgpu names it, and what it offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adapter {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub backend: String,
    /// The compiler a backend with a choice of them ran: `fxc` or `dxc` for DX12.
    #[serde(default)]
    pub shader_compiler: String,
    pub features: Vec<String>,
    pub limits: BTreeMap<String, u64>,
}

/// What the pipeline cache of an adapter saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// The file's name in the cache folder, which is wgpu's key for the adapter.
    pub file: String,
    pub bytes: usize,
    pub pipelines: usize,
    /// The milliseconds the pipelines took with an empty cache, and with the saved one loaded.
    pub empty_ms: f64,
    pub loaded_ms: f64,
}

/// One container's programs as the translator left them, before any device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    pub programs: usize,
    pub pairs: usize,
    /// The units that reached no device, by why.
    pub front: BTreeMap<String, Vec<String>>,
    /// The units the check changed before a device saw them, by what it did.
    #[serde(default)]
    pub notes: BTreeMap<String, Vec<String>>,
}

/// One container on one adapter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// The SHA-1 of the container, which keys [`Report::containers`].
    pub container: String,
    pub adapter: Adapter,
    /// The mebibytes of stack the translator and naga's writers ran on.
    #[serde(default)]
    pub stack_mib: usize,
    /// The ranges of the container's units this run checked: a long check is run a range at a time.
    #[serde(default)]
    pub units: Vec<(usize, usize)>,
    pub created: usize,
    /// The units the device refused, by the refusal with its numbers taken out.
    pub rejected: BTreeMap<String, Vec<String>>,
    /// The units not tried, by the device feature they need.
    pub skipped: BTreeMap<String, Vec<String>>,
    /// The units that ended the process inside the driver twice, by its exit status.
    pub crashed: BTreeMap<String, Vec<String>>,
    pub timed_out: Vec<String>,
    /// The milliseconds the created units took, summed: the driver's compilation where nothing had cached it.
    pub created_ms: f64,
    pub cache: Option<Cache>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub containers: BTreeMap<String, Container>,
    pub runs: Vec<Run>,
}

/// A refusal with what differs from program to program taken out, so that one cause counts once:
/// a number that stands alone becomes `N`, and the path a compiler names its source by is dropped.
pub fn reason(message: &str) -> String {
    // FXC names the source `<working directory>Shader@0x…(line,column)` before each diagnostic,
    // after a colon and a space or the bar that joins two lines.
    let mut pieces = message.split("Shader@");
    let mut message = pieces.next().unwrap_or_default().to_owned();
    for piece in pieces {
        let kept = (message.rfind(": ").map(|at| at + 2))
            .max(message.rfind(" | ").map(|at| at + 3))
            .unwrap_or(0);
        message.truncate(kept);
        message.push_str("Shader@");
        message.push_str(piece);
    }
    // DXC quotes the line of source after its error, and no two programs share one.
    let message = match message.find("DXC compile error").and(message.find(" | ")) {
        Some(end) => message[..end].to_owned(),
        None => message,
    };
    let mut out = String::new();
    let mut chars = message.chars().peekable();
    // Whether the last character was of a word: `Texture2D` and `X3004` keep their digits.
    let mut word = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() && !word {
            let hex = c == '0' && chars.peek() == Some(&'x');
            if hex {
                chars.next();
            }
            while (chars.peek()).is_some_and(|d| d.is_ascii_digit() || hex && d.is_ascii_hexdigit())
            {
                chars.next();
            }
            out.push('N');
            word = true;
        } else {
            out.push(c);
            word = c.is_alphanumeric() || c == '_';
        }
    }
    const LONGEST: usize = 240;
    match out.char_indices().nth(LONGEST) {
        Some((end, _)) => format!("{}…", &out[..end]),
        None => out,
    }
}

/// The device feature or shader capability a refusal says the device lacks.
pub fn missing_feature(message: &str) -> Option<String> {
    // wgpu: "Features Features { features_wgpu: FeaturesWGPU(X), .. } are required but not enabled on the device"
    // naga: "Capability Capabilities(X) is not supported" and "… requires Capabilities(X)".
    for (open, wanted) in [
        ("FeaturesWGPU(", "required but not enabled"),
        ("FeaturesWebGPU(", "required but not enabled"),
        ("Capabilities(", ""),
    ] {
        let Some(at) = message.find(open).filter(|_| message.contains(wanted)) else {
            continue;
        };
        let name = &message[at + open.len()..];
        let name = &name[..name.find(')').unwrap_or(name.len())];
        if !name.is_empty() && name != "0x0" {
            return Some(name.to_owned());
        }
    }
    None
}

impl Run {
    /// Counts `unit`'s outcome.
    pub fn count(&mut self, unit: &str, outcome: &Outcome, ms: f64) {
        let unit = unit.to_owned();
        match outcome {
            Outcome::Created => {
                self.created += 1;
                self.created_ms += ms;
            }
            Outcome::Rejected(why) => match missing_feature(why) {
                Some(feature) => self.skipped.entry(feature).or_default().push(unit),
                None => self.rejected.entry(reason(why)).or_default().push(unit),
            },
            Outcome::Skipped(feature) => {
                self.skipped.entry(feature.clone()).or_default().push(unit);
            }
            Outcome::Crashed(status) => self.crashed.entry(status.clone()).or_default().push(unit),
            Outcome::TimedOut => self.timed_out.push(unit),
        }
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected.values().map(Vec::len).sum()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.values().map(Vec::len).sum()
    }

    pub fn crashed_count(&self) -> usize {
        self.crashed.values().map(Vec::len).sum()
    }

    /// Whether no range of `other` holds a unit a range of this run holds.
    fn apart_from(&self, other: &Run) -> bool {
        let apart = |a: &(usize, usize), b: &(usize, usize)| a.1 <= b.0 || b.1 <= a.0;
        !self.units.is_empty()
            && !other.units.is_empty()
            && (self.units.iter()).all(|a| other.units.iter().all(|b| apart(a, b)))
    }

    /// Adds `other`, a run of other units of the same container on the same adapter.
    fn add(&mut self, other: Run) {
        self.units.extend(other.units);
        self.units.sort_unstable();
        self.created += other.created;
        self.created_ms += other.created_ms;
        self.timed_out.extend(other.timed_out);
        for (mine, theirs) in [
            (&mut self.rejected, other.rejected),
            (&mut self.skipped, other.skipped),
            (&mut self.crashed, other.crashed),
        ] {
            for (why, units) in theirs {
                mine.entry(why).or_default().extend(units);
            }
        }
        self.cache = self.cache.take().or(other.cache);
    }

    /// What tells this run from another: the container, the adapter, its backend and its driver.
    fn key(&self) -> [&str; 6] {
        let a = &self.adapter;
        [
            &self.container,
            &a.name,
            &a.backend,
            &a.shader_compiler,
            &a.driver,
            &a.driver_info,
        ]
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    #[error("a report of layout {0} does not merge with this tool's layout {VERSION}")]
    Version(u32),
}

/// `reports` as one: every run of each. Two runs of one container on one adapter, backend and
/// driver become one: added where they checked different units, the later one kept otherwise.
pub fn merge(reports: Vec<Report>) -> Result<Report, MergeError> {
    let mut out = Report {
        version: VERSION,
        ..Default::default()
    };
    for report in reports {
        if report.version != VERSION {
            return Err(MergeError::Version(report.version));
        }
        for (digest, container) in report.containers {
            let mine = out.containers.entry(digest).or_default();
            (mine.programs, mine.pairs) = (container.programs, container.pairs);
            for (mine, theirs) in [
                (&mut mine.front, container.front),
                (&mut mine.notes, container.notes),
            ] {
                for (why, units) in theirs {
                    let all = mine.entry(why).or_default();
                    all.extend(units);
                    all.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
                    all.dedup();
                }
            }
        }
        for run in report.runs {
            match out.runs.iter_mut().find(|r| r.key() == run.key()) {
                Some(same) if same.apart_from(&run) => same.add(run),
                Some(same) => *same = run,
                None => out.runs.push(run),
            }
        }
    }
    Ok(out)
}

/// The units printed after a reason, at most.
const UNITS_SHOWN: usize = 12;

/// `report` as a table: a row per run, then each refusal with the runs it came from.
pub fn table(report: &Report) -> String {
    let mut out = String::new();
    for (digest, container) in &report.containers {
        let stopped: usize = container.front.values().map(Vec::len).sum();
        let _ = writeln!(
            out,
            "container {}: {} programs, {} vertex and pixel pairs, {stopped} unit(s) reached no device",
            &digest[..digest.len().min(12)],
            container.programs,
            container.pairs
        );
        let mut reasons: Vec<_> = container.front.iter().collect();
        reasons.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
        for (why, units) in reasons {
            let _ = writeln!(out, "  {:>6}  {why}", units.len());
        }
        for (what, units) in &container.notes {
            let _ = writeln!(out, "  {:>6}  noted: {what}", units.len());
        }
    }
    let _ = writeln!(
        out,
        "\n{:<12} {:<44} {:<8} {:<34} {:>7} {:>8} {:>7} {:>7} {:>8}",
        "container",
        "adapter",
        "backend",
        "driver",
        "created",
        "rejected",
        "skipped",
        "crashed",
        "timedout"
    );
    for run in &report.runs {
        let a = &run.adapter;
        let _ = writeln!(
            out,
            "{:<12} {:<44} {:<8} {:<34} {:>7} {:>8} {:>7} {:>7} {:>8}",
            &run.container[..run.container.len().min(12)],
            a.name,
            format!("{} {}", a.backend, a.shader_compiler),
            format!("{} {}", a.driver, a.driver_info),
            run.created,
            run.rejected_count(),
            run.skipped_count(),
            run.crashed_count(),
            run.timed_out.len()
        );
    }
    for (title, pick) in [
        (
            "rejected, by reason",
            (|r: &Run| &r.rejected) as fn(&Run) -> &BTreeMap<String, Vec<String>>,
        ),
        ("skipped, by the feature the device lacks", |r: &Run| {
            &r.skipped
        }),
        ("ended the process twice, by its exit status", |r: &Run| {
            &r.crashed
        }),
    ] {
        let reasons: BTreeSet<&String> = report.runs.iter().flat_map(|r| pick(r).keys()).collect();
        if reasons.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n{title}:");
        let mut reasons: Vec<(usize, &String)> = (reasons.into_iter())
            .map(|why| {
                let most = report.runs.iter().filter_map(|r| pick(r).get(why));
                (most.map(Vec::len).max().unwrap_or(0), why)
            })
            .collect();
        reasons.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        for (_, why) in reasons {
            let _ = writeln!(out, "  {why}");
            for run in &report.runs {
                let Some(units) = pick(run).get(why) else {
                    continue;
                };
                let shown: Vec<&str> = (units.iter().take(UNITS_SHOWN))
                    .map(String::as_str)
                    .collect();
                let more = match units.len().saturating_sub(UNITS_SHOWN) {
                    0 => String::new(),
                    n => format!(" and {n} more"),
                };
                let _ = writeln!(
                    out,
                    "    {:>6}  {} ({}{}): {}{more}",
                    units.len(),
                    run.adapter.name,
                    run.adapter.backend,
                    run.adapter.shader_compiler,
                    shown.join(" ")
                );
            }
        }
    }
    for run in &report.runs {
        if let Some(cache) = &run.cache {
            let _ = writeln!(
                out,
                "\npipeline cache, {} ({}): {} pipelines, {} bytes; {:.0} ms with an empty cache, {:.0} ms with the saved one",
                run.adapter.name,
                run.adapter.backend,
                cache.pipelines,
                cache.bytes,
                cache.empty_ms,
                cache.loaded_ms
            );
        }
    }
    out
}

#[cfg(test)]
mod tests;
