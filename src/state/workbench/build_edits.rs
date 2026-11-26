//! What the Builds window changes about the builds a project declares, one edit at a time, and why a content cannot ship in a kind of build.

use or_project::{cannot_ship, Contribution, ModuleKind, Output, OutputKind, Target};

use super::Question;
use crate::state::AppState;

/// The kinds of build there are to ship in, as the matrix draws its columns.
pub const KINDS: [OutputKind; 3] = [
    OutputKind::FivemResource,
    OutputKind::SingleplayerDlc,
    OutputKind::OivPackage,
];

/// The id of the build the probe plans for the locks of the matrix, which no project declares.
const PROBE: &str = ".probe";

/// The build being added in the Builds window, as chosen so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBuild {
    pub name: String,
    /// The name field has not yet taken the keyboard.
    pub focus: bool,
    /// The name was typed, so it no longer follows the kind and the edition.
    pub named: bool,
    /// Something was changed, so a reason it cannot be added is given.
    pub edited: bool,
    pub kind: OutputKind,
    pub target: Option<Target>,
    /// The ids of the content the build takes.
    pub take: Vec<String>,
}

/// Why a kind of build cannot ship one content of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub content: String,
    pub kind: OutputKind,
    pub why: String,
    /// For a singleplayer DLC, the files of the content the game is told no type for.
    pub untyped: Vec<String>,
}

/// The locks of the matrix: for each kind of build, what each content hands a build of that kind that takes everything, and why it cannot ship in it.
pub(super) fn locks_of(
    container: &or_project::Container,
    folder: &std::path::Path,
    held: &[or_project::Source],
    folders: &[or_project::SourceFolder],
    clothing: &[Contribution],
) -> Vec<Lock> {
    let every: Vec<&str> = container.modules.iter().map(|m| m.id.as_str()).collect();
    let probe_of = |kind: &OutputKind| {
        let mut probe = container.clone();
        probe.outputs = vec![Output::of(kind.clone(), PROBE, PROBE, &every)];
        probe
    };
    let resource = probe_of(&OutputKind::FivemResource);
    let mut files = or_project::files_contributions_with(&resource, PROBE, folder, held, folders)
        .unwrap_or_default();
    files.extend(clothing.iter().cloned());
    // ⚠ A clothing pack is laid out for the kind of build it is planned for and ships in every kind, so no cell of the matrix is locked for it.
    let packs: Vec<&str> = (container.modules.iter())
        .filter(|m| m.kind == or_project::ModuleKind::Clothing)
        .map(|m| m.id.as_str())
        .collect();
    let mut locks = Vec::new();
    for kind in KINDS {
        // ⚠ A content may hand its files over as the kind of build lays them out, as a Map content hands a story pack its maps
        // inside an archive, so each kind is asked for itself and a cell says what a plan of that kind of build says.
        let probe = probe_of(&kind);
        let mut made = files.clone();
        made.extend(or_project::content::contributions_to(
            &or_content::kinds(),
            &probe,
            folder,
            &files,
            probe.outputs.first(),
        ));
        made.retain(|c| !packs.contains(&c.module.as_str()));
        for contribution in &made {
            if let Some(why) = cannot_ship(contribution, &kind) {
                let untyped = match kind {
                    OutputKind::SingleplayerDlc => or_project::files_without_a_type(contribution),
                    _ => Vec::new(),
                };
                locks.push(Lock {
                    content: contribution.module.clone(),
                    kind: kind.clone(),
                    why,
                    untyped,
                });
            }
        }
    }
    locks
}

impl AppState {
    /// Open the Builds window on the build the Build button builds, with a new build being named when `adding`.
    pub fn ask_about_builds(&mut self, adding: bool) {
        let adding = adding.then(|| self.new_build_draft());
        self.workbench.asking = Some(Question::Builds {
            chosen: self.active_output_id(),
            adding,
        });
    }

    /// The new build the Builds window offers first: a FiveM resource named for the project, taking all the content.
    pub fn new_build_draft(&self) -> NewBuild {
        let (kind, target) = (OutputKind::FivemResource, self.default_edition());
        NewBuild {
            name: self.default_build_name(&kind, target),
            focus: true,
            named: false,
            edited: false,
            kind,
            target,
            take: (self.project_carried.container.modules.iter())
                .map(|m| m.id.clone())
                .collect(),
        }
    }

    /// Change the build `id` as `change` says, and note `what` it did in the project's history; a refusal says why and changes nothing.
    pub fn edit_build(
        &mut self,
        id: &str,
        what: String,
        change: impl FnOnce(&mut Output),
    ) -> Result<(), String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        self.project_carried.container.edit_build(id, change)?;
        self.workbench.findings_stale = true;
        self.workbench.want_scan();
        self.record_step(what);
        Ok(())
    }

    /// As [`Self::edit_build`] for a number still being dragged or typed: the build changes now, and the step and the rescan wait for [`Self::settle_build_edit`].
    ///
    /// ⚠ A drag changes the number on every frame, so a step per change would push every earlier step out of the history.
    /// `ceiling` names what is changed; a later change of the same one in the same build replaces the line to be said.
    pub fn edit_build_live(
        &mut self,
        id: &str,
        ceiling: &str,
        what: String,
        change: impl FnOnce(&mut Output),
    ) -> Result<(), String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        self.project_carried.container.edit_build(id, change)?;
        self.project_carried.changed = true;
        self.workbench
            .unsaid
            .insert(format!("{id} {ceiling}"), what);
        Ok(())
    }

    /// Keep what [`Self::edit_build_live`] left unsaid as one step, and ask for the rescan it put off; nothing when none is waiting.
    pub fn settle_build_edit(&mut self) {
        let unsaid = std::mem::take(&mut self.workbench.unsaid);
        if unsaid.is_empty() {
            return;
        }
        self.workbench.findings_stale = true;
        self.workbench.want_scan();
        let said: Vec<String> = unsaid.into_values().collect();
        self.record_step(said.join("; "));
    }

    /// Ask the build `id` to be built a second time and compared byte for byte whenever it is built, or stop asking.
    pub fn verify_build(&mut self, id: &str, on: bool) -> Result<(), String> {
        let name = (self.project_carried.container.output(id))
            .map_or_else(|| id.to_string(), |o| o.name.clone());
        let what = match on {
            true => tr!(
                "The build {name} is verified with a second build",
                name = name
            ),
            false => tr!(
                "The build {name} is no longer verified with a second build",
                name = name
            ),
        };
        self.edit_build(id, what, |o| {
            match on {
                true => o
                    .extra
                    .insert(super::VERIFY.to_string(), serde_json::Value::Bool(true)),
                false => o.extra.remove(super::VERIFY),
            };
        })
    }

    /// Make the build `build` take the content `content`, or leave it out.
    pub fn take_content(&mut self, build: &str, content: &str, taken: bool) -> Result<(), String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        self.project_carried
            .container
            .set_taken(build, content, taken)?;
        self.workbench.findings_stale = true;
        self.workbench.want_scan();
        let what = if taken {
            tr!("takes")
        } else {
            tr!("leaves out")
        };
        let container = &self.project_carried.container;
        let (build, content) = (
            super::build_name(container, build),
            super::content_name(container, content),
        );
        self.record_step(tr!(
            "The build {build} {what} {content}",
            build = build,
            what = what,
            content = content
        ));
        Ok(())
    }

    /// Declare a build that takes only the content in `take`; its id.
    pub fn add_build_taking(
        &mut self,
        name: &str,
        kind: OutputKind,
        target: Option<Target>,
        take: &[String],
    ) -> Result<String, String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        if let Some(ghost) = take
            .iter()
            .find(|id| self.project_carried.container.module(id).is_none())
        {
            return Err(tr!("the project holds no content `{ghost}`", ghost = ghost));
        }
        let id = self.declare_build(name, kind, target)?;
        self.project_carried
            .container
            .edit_build(&id, |output| output.take = take.to_vec())?;
        self.record_step(tr!("Added the build {name}", name = name));
        Ok(id)
    }

    /// State the type a singleplayer DLC registers the file `path` as, or take the statement away.
    pub fn state_file_type(&mut self, path: &str, file_type: Option<&str>) -> Result<(), String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        let files = (self.project_carried.container.modules.iter_mut())
            .find(|m| m.kind == ModuleKind::Files && !m.is_link())
            .ok_or(tr!("The project holds no Files content of its own"))?;
        files.set_file_type(path, file_type)?;
        self.workbench.findings_stale = true;
        self.workbench.want_scan();
        self.record_step(match file_type {
            Some(file_type) => tr!(
                "{path} is registered as {file_type}",
                path = path,
                file_type = file_type
            ),
            None => tr!("{path} states no type", path = path),
        });
        Ok(())
    }

    /// The Files content of the project that states file types for its own files.
    pub fn own_files_content(&self) -> Option<&or_project::Module> {
        (self.project_carried.container.modules.iter())
            .find(|m| m.kind == ModuleKind::Files && !m.is_link())
    }
}

impl super::Workbench {
    /// The files of the content `content` that a singleplayer DLC is told no type for, as the last scan found.
    pub fn untyped_files(&self, content: &str) -> &[String] {
        let Some(scan) = self.scan.as_ref() else {
            return &[];
        };
        (scan.locks.iter())
            .find(|lock| lock.content == content && lock.kind == OutputKind::SingleplayerDlc)
            .map_or(&[], |lock| lock.untyped.as_slice())
    }

    /// Why the build kind `kind` cannot ship the content `content`, as the last scan found; none when it can.
    pub fn cannot_ship(&self, content: &str, kind: &OutputKind) -> Option<&str> {
        let scan = self.scan.as_ref()?;
        (scan.locks.iter())
            .find(|lock| lock.content == content && &lock.kind == kind)
            .map(|lock| lock.why.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_maps::import::{add_files, Incoming};
    use or_maps::sample::{map_file, types_file, Archetype, Entity};
    use or_project::{Container, Module};

    /// ⚠ A Map content hands a story pack its maps inside an archive and a resource the same maps loose, so the sentence of a cell
    /// is the one of the build the cell is for: what `project plan` of that build says, never what a resource's layout would.
    #[test]
    fn the_cell_of_a_kind_of_build_is_judged_by_what_the_content_hands_that_kind() {
        let dir = tempfile::tempdir().unwrap();
        let placed = map_file(
            "harbour",
            &[Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)],
            ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
        );
        let declared = types_file(
            "harbour_types",
            &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
        );
        let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        };
        let files = [
            incoming("harbour.ymap", placed.leak()),
            incoming("harbour_types.ytyp", declared.leak()),
        ];
        let added = add_files(&dir.path().join("maps"), &files, None).unwrap();
        assert!(added.refused.is_empty(), "{:?}", added.refused);
        let container = Container {
            modules: vec![Module::maps("maps", "maps")],
            outputs: Vec::new(),
        };

        let locks = locks_of(&container, dir.path(), &[], &[], &[]);
        let of = |kind: OutputKind| {
            (locks.iter())
                .find(|lock| lock.content == "maps" && lock.kind == kind)
                .map(|lock| lock.why.as_str())
        };
        assert_eq!(of(OutputKind::FivemResource), None);
        let said = of(OutputKind::SingleplayerDlc).expect("the cell of a story pack is locked");
        let archive = or_maps::layout::pack_archive("maps");
        assert!(
            said.contains(&archive) && !said.contains("stream/harbour.ymap"),
            "the cell speaks of the archive a story pack holds the maps in: {said}"
        );
    }
}
