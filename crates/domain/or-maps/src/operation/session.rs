//! The sources of a pack of maps held open, so that an edit costs what a screen can afford: each source an operation edits is kept as
//! its typed root, an operation changes that root and writes nothing, and [`Session::flush`] writes the XML of every source that
//! changed. A drag of many steps writes once.
//!
//! ⚠ **A session that refused an operation does not write when it is dropped.** A session dropped with operations unwritten writes
//! them, but not once [`Session::apply`] refused one since the last flush: what was done before the refusal may be the first
//! operations of a list that is sound only whole. [`Session::apply_all`] does such a list or none of it, and a caller that wants
//! what was done before a refusal written flushes.
//!
//! ⚠ **What a flush writes is what [`super::apply`] writes, operation by operation**, byte for byte, and each source is written through
//! the same guard. What a save or the guard refuses is refused at the flush and not at the operation, because finding it out is the
//! cost a session is there to pay once: the flush then writes nothing and names the first operation it could not write.
//!
//! ⚠ **A state no XML holds has no digest of an XML.** The inverse a session gives states a token of that session for the state it
//! applies to, which only that session accepts, and which moves back and forward with the inverses applied in order. What a flush
//! wrote has the digest [`Written`] states, which the session accepts beside its token and anything that reads the disk accepts alone.
//!
//! ⚠ **An inverse is given without a save where it holds by construction** (`sure_in_map`, `sure_in_types`), and the
//! flush proves the ones given since the last: taken back newest first, they save the file the source saved then. Where it does not
//! hold by construction the operation is done as [`super::apply`] does it, with the save, the XML and the proof, and where the
//! inverse is then the XML the root is read again from the XML written, so a session saves nothing the source read again would not.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use or_io::decl::File;
use or_io::meta::typed::{CBaseArchetypeDefAny, CEntityDefAny, CMapData, CMapTypes};
use or_source::metadata::Inputs;
use sha2::{Digest, Sha256};

use super::{
    archetype_at, change_map, change_types, context, declare, digest_of, entity_at, opened,
    problems, proven, refused, restores, settle_notes, sure_in_map, sure_in_types, text_of,
    write_all, At, Boxes, Changed, Context, Error, Linked, Operation, Pack,
};
use crate::model::Maps;

/// How many of the inverses it gave a source knows the way of: a thousand operations back and forward again. An older one is done
/// as an operation like any other, and the ones given before it are then refused.
const TRAIL: usize = 2048;

/// What an operation did to a session, which has written nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// The operation that puts the source back, which states the state this one left: a token of the session until a flush writes it.
    pub inverse: Operation,
    /// The index of the entity or the archetype it added, when it added one.
    pub added: Option<usize>,
    /// What it could not read and went without, a sentence each, as [`super::Applied::notes`] says it.
    pub notes: Vec<String>,
}

/// A source a flush wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The source, as the pack names it.
    pub source: String,
    /// The XML file of the source.
    pub file: PathBuf,
    /// The digest of the XML written: what an operation states to be applied to it by anything that reads the disk.
    pub digest: String,
    /// Whether the inverses given since the source was last written were seen to put it back; when not, each of them is refused from here on.
    pub proven: bool,
}

/// The file a source cooks to, as the typed root an operation changes.
#[derive(Clone)]
enum Root {
    Map(Box<CMapData>),
    Types(Box<CMapTypes>),
}

impl Root {
    fn open(inputs: &Inputs) -> Result<Self, String> {
        let map = match inputs.extension() {
            CMapData::EXTENSION => true,
            CMapTypes::EXTENSION => false,
            other => {
                return Err(format!(
                    "it is a .{other} source, and an operation edits a map file or a type file"
                ))
            }
        };
        let cooked = inputs.compile().map_err(problems)?.bytes;
        if map {
            CMapData::from_bytes(&cooked).map(|map| Self::Map(Box::new(map)))
        } else {
            CMapTypes::from_bytes(&cooked).map(|types| Self::Types(Box::new(types)))
        }
        .map_err(|e| e.to_string())
    }

    fn save(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Map(map) => map.to_bytes(),
            Self::Types(types) => types.to_bytes(),
        }
        .map_err(|e| e.to_string())
    }

    /// Do `operation` to the root; a refusal leaves it as it was.
    fn change(&mut self, operation: &Operation, context: &Context) -> Result<Changed, String> {
        match self {
            Self::Map(map) => change_map(map, operation, context),
            Self::Types(types) => change_types(types, operation, context),
        }
    }

    /// Whether the inverse of `operation` puts the root back as it is with nothing to prove.
    fn sure(&self, operation: &Operation) -> bool {
        match self {
            Self::Map(map) => sure_in_map(map, operation),
            Self::Types(types) => sure_in_types(types, operation),
        }
    }

    /// `inverse` as its JSON reads back, once doing that to the root saves the bytes `was`.
    fn proven(&self, inverse: &Operation, was: &[u8]) -> Option<Operation> {
        match self {
            Self::Map(map) => proven(&**map, inverse, was, change_map),
            Self::Types(types) => proven(&**types, inverse, was, change_types),
        }
    }

    /// Whether `inverse`, done to the source read again once `inputs` hold `xml`, writes the XML `inputs` hold.
    fn restores(&self, inputs: &Inputs, xml: &[u8], inverse: &Operation) -> bool {
        match self {
            Self::Map(_) => restores::<CMapData>(inputs, xml, inverse, change_map),
            Self::Types(_) => restores::<CMapTypes>(inputs, xml, inverse, change_types),
        }
    }

    /// The root with the operations of `done` taken back, newest first; none when an inverse is refused.
    fn before(&self, done: &[Pending]) -> Option<Self> {
        let mut back = self.clone();
        for step in done.iter().rev() {
            back.change(&step.inverse, &Context::default()).ok()?;
        }
        Some(back)
    }
}

/// The SHA-256 of an operation's JSON, by which an operation given back is known as the one that was given; none when it has no JSON.
///
/// ⚠ Of the operation with no digest stated: the state it finds is checked apart, and an inverse given with a token is the same
/// inverse once it states the digest a flush gave.
type Mark = Option<[u8; 32]>;

fn mark(operation: &Operation) -> Mark {
    let mut bare = operation.clone();
    bare.set_expects(None);
    let json = serde_json::to_vec(&bare).ok()?;
    Some(Sha256::digest(json).into())
}

/// Whether two marks are of one operation: an operation with no JSON is no other's.
fn same(one: Mark, other: Mark) -> bool {
    one.is_some() && one == other
}

/// An operation done to a root since its XML was last settled.
#[derive(Clone)]
struct Pending {
    /// The name of the operation, which a flush that cannot write it says.
    name: &'static str,
    /// What takes it back, which needs nothing read besides the root.
    inverse: Operation,
    undoes: Mark,
}

/// A source the session has opened.
struct Held {
    dir: PathBuf,
    /// The inputs, whose XML is the source as it was last read, rewritten or settled.
    inputs: Inputs,
    /// The XML the disk holds.
    on_disk: Vec<u8>,
    /// The digest of the XML of `inputs`.
    content: String,
    /// The file the XML cooks to with every operation since done to it, opened when an operation first needs it.
    root: Option<Root>,
    /// What the root saved as when the XML was settled; none when it did not save or the XML was not its export, so that only the XML puts it back.
    reference: Option<Vec<u8>>,
    /// The operations done to the root since the XML was settled, oldest first: each with an inverse that holds by construction.
    pending: Vec<Pending>,
    /// The name of the state the root is in: the digest of an XML, or a token of this session for a state no XML holds yet.
    state: String,
    /// Where each inverse given leads: from the state it applies to, by its mark, to the state its operation was done to.
    leads: BTreeMap<(String, [u8; 32]), String>,
    /// The keys of `leads`, oldest first.
    given: VecDeque<(String, [u8; 32])>,
    /// Pairs of names of one state: where one operation, done to one state, was seen to lead.
    alike: VecDeque<(String, String)>,
    /// A settle found the inverses not to put the root back, and no flush has said so yet.
    unproven: bool,
}

impl Held {
    /// Whether an operation that states `expects` applies to the state held.
    fn is(&self, expects: &str) -> bool {
        self.names_of(&self.state).contains(&expects)
            || (self.pending.is_empty() && expects == self.content)
    }

    /// Every name of the state `name` names, `name` first.
    fn names_of<'a>(&'a self, name: &'a str) -> Vec<&'a str> {
        let mut names = vec![name];
        let mut at = 0;
        while at < names.len() {
            for (one, other) in &self.alike {
                for (from, to) in [(one, other), (other, one)] {
                    if from == names[at] && !names.contains(&to.as_str()) {
                        names.push(to);
                    }
                }
            }
            at += 1;
        }
        names
    }

    /// Where the operation of mark `by` leads from the state `from`, when it is an inverse given for that state.
    fn led(&self, from: &str, by: [u8; 32]) -> Option<&String> {
        (self.names_of(from).into_iter()).find_map(|name| self.leads.get(&(name.to_string(), by)))
    }

    /// The root, opened from the XML when it is not held.
    fn open_root(&mut self) -> Result<&mut Root, String> {
        if self.root.is_none() {
            let root = Root::open(&self.inputs)?;
            let saved = root.save().ok();
            self.reference = saved.filter(|saved| self.inputs.is_the_export_of(saved));
            self.root = Some(root);
        }
        self.root
            .as_mut()
            .ok_or_else(|| "the source was not opened".to_string())
    }

    /// The root, of a source whose file has the extension `asked`.
    fn root_as(&mut self, asked: &str) -> Result<&Root, String> {
        let extension = self.inputs.extension();
        if extension != asked {
            return Err(format!(
                "it is a .{extension} source, and a .{asked} is asked of it"
            ));
        }
        self.open_root().map(|root| &*root)
    }

    /// Record that `operation` was done and `inverse` takes it back; the inverse with the state it applies to.
    ///
    /// ⚠ An inverse the session gave leads back to the state its operation was done to, and the inverse of that one forward again, so
    /// that inverses applied in order each find the state they state. It is known by the state it is done to as well as by what it
    /// is: the step taken back from a state and the step made again from it are one operation when the states either side of it are
    /// alike, an entity put back by hand where it stood, and each leads where its own step led.
    fn note(
        &mut self,
        operation: &Operation,
        inverse: Operation,
        settled: bool,
        token: String,
    ) -> Operation {
        let did = mark(operation);
        // The state the operation left, which its inverse applies to.
        let left = match did.and_then(|did| self.led(&self.state, did)) {
            Some(known) => known.clone(),
            None if settled => self.content.clone(),
            None => token,
        };
        let inverse = inverse.expecting(left.clone());
        let given = mark(&inverse);
        if let Some(by) = given {
            match self.led(&left, by).cloned() {
                // ⚠ One operation done to one state leaves one state, so the two names are of one.
                Some(to) => {
                    if !self.names_of(&to).contains(&self.state.as_str()) {
                        self.alike.push_back((to, self.state.clone()));
                    }
                }
                None => {
                    self.leads.insert((left.clone(), by), self.state.clone());
                    self.given.push_back((left.clone(), by));
                }
            }
            if self.given.len() > TRAIL {
                if let Some(oldest) = self.given.pop_front() {
                    self.leads.remove(&oldest);
                }
            }
            if self.alike.len() > TRAIL {
                self.alike.pop_front();
            }
        }
        self.state = left;
        if !settled {
            // An operation and the inverse given for it leave the root as it was: neither is left to write.
            if (self.pending.last()).is_some_and(|last| same(last.undoes, did)) {
                self.pending.pop();
            } else {
                self.pending.push(Pending {
                    name: operation.name(),
                    inverse: inverse.clone(),
                    undoes: given,
                });
            }
        }
        inverse
    }

    /// Why the root cannot be written, with the first operation after which it could not be.
    fn unwritten(&self, source: &str, why: String) -> Error {
        let of = self.pending.len();
        // ⚠ Halved, on the reading that a root that cannot be written stays so: the root as it was settled can be, and as it is cannot.
        let (mut can, mut cannot) = (0, of);
        while cannot - can > 1 {
            let middle = can + (cannot - can) / 2;
            let writes = (self.root.as_ref())
                .and_then(|root| root.before(&self.pending[middle..]))
                .and_then(|root| root.save().ok())
                .is_some_and(|saved| self.inputs.xml_of(&saved).is_ok());
            if writes {
                can = middle;
            } else {
                cannot = middle;
            }
        }
        Error::Unwritten {
            at: source.to_string(),
            why,
            number: cannot,
            of,
            name: (self.pending.get(cannot.saturating_sub(1)))
                .map_or("operation", |step| step.name),
        }
    }

    /// Write the root out as the XML the inputs hold, nothing reaching the disk, and prove the inverses given since the last time.
    fn settle(&mut self, source: &str) -> Result<(), Error> {
        let (Some(root), false) = (&self.root, self.pending.is_empty()) else {
            return Ok(());
        };
        let written = root.save().and_then(|saved| {
            let xml = self.inputs.xml_of(&saved).map_err(problems)?;
            Ok((saved, xml))
        });
        let (saved, xml) = match written {
            Ok(written) => written,
            Err(why) => return Err(self.unwritten(source, why)),
        };
        let put_back = (self.reference.as_deref()).is_some_and(|was| {
            (root.before(&self.pending))
                .is_some_and(|back| back.save().ok().as_deref() == Some(was))
        });
        self.content = digest_of(&xml);
        if !put_back {
            tracing::error!(
                source,
                "the inverses given for the operations done to a map source do not put it back, and are refused from here on"
            );
            self.unproven = true;
            self.leads.clear();
            self.given.clear();
            self.alike.clear();
            self.state = self.content.clone();
        }
        self.inputs.set_xml(xml);
        self.reference = Some(saved);
        self.pending.clear();
        Ok(())
    }
}

/// Counts the sessions of the process, so that two opened at one instant have tokens of their own.
static SESSIONS: AtomicU64 = AtomicU64::new(0);

/// What makes the tokens of one session no other's: the process, the instant and the count of sessions.
fn nonce() -> String {
    let mut hash = Sha256::new();
    hash.update(std::process::id().to_le_bytes());
    hash.update(SESSIONS.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    hash.update(now.to_le_bytes());
    (hash.finalize().iter().take(8))
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The sources of a pack of maps held open as typed roots: operations change the roots, and [`Session::flush`] writes them.
pub struct Session {
    folder: PathBuf,
    /// The map sets of the pack, as they were when the session opened.
    model: Maps,
    held: BTreeMap<String, Held>,
    /// The boxes of the archetypes the sets of a map source declare, with a sentence for each type file that does not cook, read once and read again after a type file is edited.
    boxes: BTreeMap<String, (Arc<Boxes>, Vec<String>)>,
    nonce: String,
    tokens: u64,
    /// Whether an operation was refused since the last flush, so that what was done before it may be part of a list that is sound only whole.
    refused: bool,
}

impl Session {
    /// A session over the sources of the pack of maps in `folder`; a source is read when an operation first names it.
    pub fn open(folder: &Path) -> Result<Self, Error> {
        Ok(Self {
            folder: folder.to_path_buf(),
            model: Maps::read(folder)?,
            held: BTreeMap::new(),
            boxes: BTreeMap::new(),
            nonce: nonce(),
            tokens: 0,
            refused: false,
        })
    }

    /// The source `source`, read once.
    fn hold(&mut self, source: &str) -> Result<&mut Held, String> {
        if !self.held.contains_key(source) {
            let (dir, inputs) = opened(&self.folder, &self.model, source)?;
            let content = digest_of(inputs.xml());
            let held = Held {
                dir,
                on_disk: inputs.xml().to_vec(),
                inputs,
                state: content.clone(),
                content,
                root: None,
                reference: None,
                pending: Vec::new(),
                leads: BTreeMap::new(),
                given: VecDeque::new(),
                alike: VecDeque::new(),
                unproven: false,
            };
            self.held.insert(source.to_string(), held);
        }
        self.held
            .get_mut(source)
            .ok_or_else(|| "the source was not opened".to_string())
    }

    fn map_of(&mut self, source: &str) -> Result<&CMapData, String> {
        match self.hold(source)?.root_as(CMapData::EXTENSION)? {
            Root::Map(map) => Ok(map),
            Root::Types(_) => Err("it is not a map file".to_string()),
        }
    }

    fn types_of(&mut self, source: &str) -> Result<&CMapTypes, String> {
        match self.hold(source)?.root_as(CMapTypes::EXTENSION)? {
            Root::Types(types) => Ok(types),
            Root::Map(_) => Err("it is not a type file".to_string()),
        }
    }

    /// The map file the source `source` cooks to, as the operations done so far leave it.
    pub fn map(&mut self, source: &str) -> Result<&CMapData, Error> {
        self.map_of(source).map_err(|why| Error::Source {
            at: source.to_string(),
            why,
        })
    }

    /// The type file the source `source` cooks to, as the operations done so far leave it.
    pub fn types(&mut self, source: &str) -> Result<&CMapTypes, Error> {
        self.types_of(source).map_err(|why| Error::Source {
            at: source.to_string(),
            why,
        })
    }

    /// Whether an operation was done that no flush has written.
    pub fn unwritten(&self) -> bool {
        (self.held.values())
            .any(|held| !held.pending.is_empty() || held.inputs.xml() != held.on_disk.as_slice())
    }

    /// Do `operation` to the root of its source and say what takes it back; nothing is written, and a refusal leaves every root as it was and keeps the session from writing when it is dropped.
    pub fn apply(&mut self, operation: &Operation) -> Result<Done, Error> {
        let done = self.apply_one(operation);
        self.refused |= done.is_err();
        done
    }

    fn apply_one(&mut self, operation: &Operation) -> Result<Done, Error> {
        let source = operation.source();
        let held = (self.hold(source)).map_err(|why| refused(operation, why))?;
        if let Some(expects) = operation.expects().filter(|expects| !held.is(expects)) {
            let found = if held.pending.is_empty() {
                &held.content
            } else {
                &held.state
            };
            return Err(Error::Stale {
                name: operation.name(),
                at: source.to_string(),
                expects: expects.to_string(),
                found: found.clone(),
            });
        }
        let (inverse, added, settled, notes) = match operation {
            Operation::RewriteSource { xml, .. } => {
                (self.rewrite(operation, xml)?, None, true, Vec::new())
            }
            _ => self.change(operation)?,
        };
        self.tokens += 1;
        let token = format!("unwritten-{}-{}", self.nonce, self.tokens);
        let held = (self.hold(source)).map_err(|why| refused(operation, why))?;
        Ok(Done {
            inverse: held.note(operation, inverse, settled, token),
            added,
            notes,
        })
    }

    /// Do `operations` in order, all of them or none, and say what each did; nothing is written, and when one is refused the ones done before it are taken back by their inverses, so every root is as it was.
    pub fn apply_all(&mut self, operations: &[Operation]) -> Result<Vec<Done>, Error> {
        let refused_before = self.refused;
        let mut done: Vec<Done> = Vec::with_capacity(operations.len());
        for (index, operation) in operations.iter().enumerate() {
            let refused = match self.apply(operation) {
                Ok(step) => {
                    done.push(step);
                    continue;
                }
                Err(refused) => Error::InList {
                    number: index + 1,
                    of: operations.len(),
                    refused: Box::new(refused),
                },
            };
            for (number, step) in done.iter().enumerate().rev() {
                if let Err(left) = self.apply(&step.inverse) {
                    return Err(Error::NotTakenBack {
                        refused: Box::new(refused),
                        number: number + 1,
                        why: left.to_string(),
                    });
                }
            }
            // The roots are as they were before the list, so the refusal leaves nothing half done for a drop to write.
            self.refused = refused_before;
            return Err(refused);
        }
        settle_notes(operations, done.iter_mut().map(|step| &mut step.notes));
        Ok(done)
    }

    /// Let the session go with nothing written: what no flush wrote is forgotten, where a session that is dropped with no refusal writes it.
    pub fn discard(mut self) {
        self.held.clear();
    }

    /// Hold `xml` as the XML of the source of `operation`; the operation that holds the XML it held.
    fn rewrite(&mut self, operation: &Operation, xml: &str) -> Result<Operation, Error> {
        let source = operation.source();
        let no = |why: String| refused(operation, why);
        let held = self.hold(source).map_err(no)?;
        // What takes it back is the XML before it, which a root with operations done to it is yet to be written as.
        held.settle(source)?;
        let was = text_of(&held.inputs).map_err(no)?;
        let mut candidate = held.inputs.clone();
        candidate.set_xml(xml.as_bytes().to_vec());
        // ⚠ XML that does not compile would be a source nothing cooks.
        candidate.compile().map_err(|found| no(problems(found)))?;
        held.inputs = candidate;
        held.content = digest_of(xml.as_bytes());
        held.root = None;
        held.reference = None;
        self.boxes.clear();
        Ok(Operation::RewriteSource {
            source: source.to_string(),
            expects: None,
            xml: was,
        })
    }

    /// Do `operation` to the root of its source; the inverse, what it added, whether the XML of the result is settled, and what it went without.
    fn change(
        &mut self,
        operation: &Operation,
    ) -> Result<(Operation, Option<usize>, bool, Vec<String>), Error> {
        let source = operation.source();
        let no = |why: String| refused(operation, why);
        let extension = self.hold(source).map_err(no)?.inputs.extension();
        if extension != CMapData::EXTENSION && extension != CMapTypes::EXTENSION {
            return Err(no(format!(
                "it is a .{extension} source, and an operation edits a map file or a type file"
            )));
        }
        let context = context(self, operation).map_err(no)?;
        let held = self.hold(source).map_err(no)?;
        let root = held.open_root().map_err(no)?;
        let types = matches!(root, Root::Types(_));
        // ⚠ What takes back the operation done last leaves the root as that one found it, whatever is yet to be written: it is how a
        // flush that was refused is answered, and nothing is saved for it.
        let sure = root.sure(operation)
            || (held.pending.last()).is_some_and(|last| same(last.undoes, mark(operation)));
        let sure = sure && held.reference.is_some();
        let done = if sure {
            let root = held.open_root().map_err(no)?;
            let (inverse, added) = root.change(operation, &context).map_err(no)?;
            (inverse, added, false)
        } else {
            // What nothing shows will be put back is done as `super::apply` does it: saved, written as XML, and the inverse proven.
            held.settle(source)?;
            let mut next = held.open_root().map_err(no)?.clone();
            let (inverse, added) = next.change(operation, &context).map_err(no)?;
            let saved = next.save().map_err(no)?;
            let xml = (held.inputs.xml_of(&saved)).map_err(|found| no(problems(found)))?;
            let proven = (held.reference.as_deref())
                .and_then(|was| next.proven(&inverse, was))
                .filter(|inverse| next.restores(&held.inputs, &xml, inverse));
            let inverse = match proven {
                Some(inverse) => {
                    held.reference = Some(saved);
                    held.root = Some(next);
                    inverse
                }
                None => {
                    let was = text_of(&held.inputs).map_err(no)?;
                    // ⚠ Read again from the XML when next asked for: the root the operation was done to may save what the source
                    // read again does not, a record of a class the XML no longer names.
                    held.reference = None;
                    held.root = None;
                    Operation::RewriteSource {
                        source: source.to_string(),
                        expects: None,
                        xml: was,
                    }
                }
            };
            held.content = digest_of(&xml);
            held.inputs.set_xml(xml);
            (inverse, added, true)
        };
        if types {
            self.boxes.clear();
        }
        let (inverse, added, settled) = done;
        Ok((inverse, added, settled, context.notes.into_inner()))
    }

    /// Write the XML of every source an operation changed, through the guard a source is written through, all of them or none, and say what was written.
    pub fn flush(&mut self) -> Result<Vec<Written>, Error> {
        // ⚠ All or none: a source the guard refuses stops the flush before anything is written, with the first operation it could
        // not write, so does a source something else wrote since the session read it, and a source the disk refuses has the ones
        // written before it put back. The roots are kept each time.
        for (source, held) in &mut self.held {
            held.settle(source)?;
        }
        let changed = |held: &Held| held.inputs.xml() != held.on_disk.as_slice();
        // ⚠ A session lives as long as a screen does, and a source something else wrote meanwhile is not the one its root was read from.
        for (source, held) in &self.held {
            let on_disk = std::fs::read(held.dir.join(or_source::metadata::XML)).ok();
            if changed(held) && on_disk.as_deref() != Some(held.on_disk.as_slice()) {
                return Err(Error::Changed { at: source.clone() });
            }
        }
        let to_write: Vec<(&Path, &[u8], &[u8])> = (self.held.values())
            .filter(|held| changed(held))
            .map(|held| {
                (
                    held.dir.as_path(),
                    held.inputs.xml(),
                    held.on_disk.as_slice(),
                )
            })
            .collect();
        write_all(&to_write)?;
        let mut written = Vec::new();
        for (source, held) in &mut self.held {
            if !changed(held) {
                continue;
            }
            held.on_disk = held.inputs.xml().to_vec();
            written.push(Written {
                source: source.clone(),
                file: held.dir.join(or_source::metadata::XML),
                digest: held.content.clone(),
                proven: !std::mem::take(&mut held.unproven),
            });
        }
        // The caller chose to write what the session held, a refusal before it or not.
        self.refused = false;
        Ok(written)
    }
}

impl Pack for Session {
    fn boxes(&mut self, source: &str) -> (Arc<Boxes>, Vec<String>) {
        if let Some(read) = self.boxes.get(source) {
            return read.clone();
        }
        let paths: Vec<String> = (self.model.maps.iter())
            .filter(|set| set.sources.iter().any(|s| s == source))
            .flat_map(|set| set.sources.iter().cloned())
            .collect();
        let mut boxes = Boxes::new();
        let mut notes = Vec::new();
        for path in paths {
            // A source of another kind, a map file or a model, declares nothing and is no one's to mention.
            let is_types = (self.hold(&path))
                .is_ok_and(|held| held.inputs.extension() == CMapTypes::EXTENSION);
            if !is_types {
                continue;
            }
            match self.types_of(&path) {
                Ok(types) => declare(&mut boxes, types),
                Err(why) => notes.push(super::does_not_cook(&path, &why)),
            }
        }
        let read = (Arc::new(boxes), notes);
        self.boxes.insert(source.to_string(), read.clone());
        read
    }

    fn entity(&mut self, at: &At) -> Result<CEntityDefAny, String> {
        (self.map_of(&at.source)).and_then(|map| entity_at(map, at.index).cloned())
    }

    fn archetype(&mut self, at: &At) -> Result<CBaseArchetypeDefAny, String> {
        (self.types_of(&at.source)).and_then(|types| archetype_at(types, at.index).cloned())
    }

    fn sources(&self) -> Vec<String> {
        super::sources_of(&self.model)
    }

    fn linked(&mut self, source: &str) -> Option<Linked> {
        let file = (self.hold(source).ok())
            .filter(|held| held.inputs.extension() == CMapData::EXTENSION)
            .map(|held| held.inputs.output_name())?;
        Some(Linked::of(self.map_of(source).ok()?, &file))
    }
}

impl Drop for Session {
    /// Writes what no flush wrote, unless an operation was refused since the last flush. A refusal can only be logged here, so a caller that has to know flushes itself.
    ///
    /// ⚠ A session that refused an operation writes nothing here: what it did before the refusal may be the first operations of a
    /// list that is sound only whole, a chain deletion among them, and a caller that stops at the refusal with `?` drops the
    /// session. Such a caller that wants them written flushes.
    fn drop(&mut self) {
        // ⚠ Not while a panic unwinds: what it interrupted may have left a root half changed.
        if !self.unwritten() || std::thread::panicking() {
            return;
        }
        if self.refused {
            tracing::error!(
                folder = %self.folder.display(),
                "a session of map edits was dropped after it refused an operation, so what it did before the refusal was not written"
            );
            return;
        }
        if let Err(refused) = self.flush() {
            tracing::error!(
                folder = %self.folder.display(),
                error = %refused,
                "a session of map edits was dropped with operations it could not write"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{add_files, Incoming};
    use crate::operation::{Extents, PoseData};
    use crate::sample::{map_file, Entity};

    const SOURCE: &str = "quay.metadata.src";

    #[test]
    fn a_flush_finds_inverses_that_do_not_put_a_source_back_and_they_are_refused_from_then_on() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join("maps");
        let quay = map_file(
            "quay",
            &[Entity::at("prop_crate_a", [1.0, 2.0, 0.5], 1)],
            ([0.0; 3], [100.0; 3]),
        );
        let file = Incoming {
            name: "quay.ymap",
            bytes: &quay,
            from_game: false,
            virtual_path: None,
        };
        add_files(&folder, &[file], None).unwrap();
        let place = |x: f32| Operation::PlaceEntity {
            source: SOURCE.into(),
            expects: None,
            entity: 0,
            pose: PoseData {
                position: [x, 2.0, 0.5],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale_xy: 1.0,
                scale_z: 1.0,
            },
        };

        let mut session = Session::open(&folder).unwrap();
        let sound = session.apply(&place(5.0)).unwrap();
        let [written] = session.flush().unwrap().try_into().unwrap();
        assert!(written.proven);

        // The inverse the session keeps is swapped for one that puts nothing back, as an inverse that was wrong would be.
        let wrong = session.apply(&place(9.0)).unwrap();
        let held = session.held.get_mut(SOURCE).unwrap();
        let extents = match held.root.as_ref().unwrap() {
            Root::Map(map) => Extents::of(map),
            Root::Types(_) => panic!("a map file"),
        };
        held.pending[0].inverse = Operation::SetExtents {
            source: SOURCE.into(),
            expects: None,
            extents,
        };
        let [written] = session.flush().unwrap().try_into().unwrap();
        assert!(
            !written.proven,
            "the flush says the inverses were not seen to hold"
        );
        assert_eq!(
            written.digest,
            digest_of(&std::fs::read(&written.file).unwrap()),
            "what was done is written all the same"
        );
        for given in [&wrong.inverse, &sound.inverse] {
            let refused = session.apply(given).unwrap_err();
            assert!(matches!(refused, Error::Stale { .. }), "{refused:?}");
        }
        // The session goes on from what it wrote.
        let next = session.apply(&place(11.0)).unwrap();
        session.apply(&next.inverse).unwrap();
        assert!(!session.unwritten());
    }
}
