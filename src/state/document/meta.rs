//! The metadata document: its text, its parsed instance, and whether it will
//! save.
//!
//! Split from [`super`] to keep that file inside the shell's code-line gate.

use super::*;

#[derive(Debug, Clone)]
pub struct ParsedMeta {
    pub instance: or_io::meta::MetaInstance,
    pub revision: u64,
}

/// The metadata schema, installed on first use from where `or_title::schema` finds it.
pub fn meta_schema() -> &'static or_io::meta::SchemaLibrary {
    static FOUND: std::sync::Once = std::sync::Once::new();
    FOUND.call_once(|| {
        let config = crate::state::prefs::config_root();
        if let Some(path) = or_title::schema::find_gta6_snapshot(config.as_deref()) {
            if let Err(error) = or_io::meta::install_names_snapshot(&path) {
                tracing::warn!(path = %path.display(), %error, "GTA VI's schema snapshot is unusable");
            }
        }
        let Some(path) = or_title::schema::find_snapshot(config.as_deref()) else {
            tracing::info!("no metadata schema snapshot found; field names read as hashes");
            return;
        };
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "the schema snapshot is unusable; field names read as hashes"
            );
        }
    });
    or_io::meta::default_library()
}

/// The name of the first element `bytes` open, without its namespace prefix, read from the first 8 KiB.
pub(super) fn peek_xml_root(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(8 * 1024)];
    let s = std::str::from_utf8(head).ok()?;
    let mut i = 0usize;
    let chars: Vec<char> = s.chars().collect();
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'?') {
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'!') {
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && !chars[j].is_whitespace() && chars[j] != '>' && chars[j] != '/' {
            j += 1;
        }
        let name: String = chars[i + 1..j].iter().collect();
        return Some(match name.rsplit_once(':') {
            Some((_, local)) => local.to_string(),
            None => name,
        });
    }
    None
}

#[cfg(test)]
#[path = "a_document_that_will_not_save.rs"]
mod a_document_that_will_not_save;

#[derive(Debug, Clone)]
pub struct MetaDocument {
    pub text: TextDocument,
    pub text_revision: u64,
    pub parsed: Option<ParsedMeta>,
    pub parse_error: Option<String>,
    pub parsed_at_text_revision: u64,
    pub text_dirty_for: Option<u64>,
    pub source_format: or_io::meta::SourceFormat,
    pub layout: Option<or_io::meta::binary::MetaLayout>,
    pub raw_bytes: Vec<u8>,
    /// Why the document would not save, and the revision that was true of.
    ///
    /// ⚠ **The validation is the save**: it keeps [`Self::to_bytes`]'s error,
    /// so it cannot disagree with the writer. Encoding is not free, so it runs
    /// once per revision rather than once per frame.
    validation: Option<(u64, Option<String>)>,
}

impl MetaDocument {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let parse_result = or_io::meta::MetaDocument::parse(meta_schema(), bytes.clone());
        Self::from_parse(bytes, parse_result)
    }

    /// A GTA VI headerless RBM resource, from its virtual and its physical section.
    pub fn from_gta_vi_headerless(virtual_section: &[u8], physical: &[u8]) -> Result<Self, String> {
        // The names of GTA VI's snapshot are installed by the first ask for the schema.
        meta_schema();
        let (instance, layout) = or_io::meta::binary::rbm::gta6::open(virtual_section, physical)
            .map_err(|e| e.to_string())?;
        let parsed = or_io::meta::MetaDocument {
            instance,
            raw_bytes: Vec::new(),
            layout: Some(layout),
            revision: 0,
            source_format: or_io::meta::SourceFormat::Rbm,
        };
        Ok(Self::from_parse(virtual_section.to_vec(), Ok(parsed)))
    }

    fn from_parse(
        bytes: Vec<u8>,
        parse_result: or_io::meta::Result<or_io::meta::MetaDocument>,
    ) -> Self {
        let (parsed, parse_error, source_format, layout) = match parse_result {
            Ok(parsed_doc) => (
                Some(ParsedMeta {
                    instance: parsed_doc.instance,
                    revision: 0,
                }),
                None,
                parsed_doc.source_format,
                parsed_doc.layout,
            ),
            Err(e) => {
                let format = match or_io::meta::detect_binary_format(&bytes) {
                    Some(fmt) => or_io::meta::SourceFormat::from_binary(fmt),
                    None => or_io::meta::SourceFormat::Xml,
                };
                (None, Some(e.to_string()), format, None)
            }
        };

        let text_bytes = match (&parsed, source_format) {
            (Some(p), fmt) if fmt.is_binary() => or_io::meta::write_meta(&p.instance)
                .map(|s| s.into_bytes())
                .unwrap_or_else(|_| bytes.clone()),
            _ => bytes.clone(),
        };
        let text = TextDocument::decode(&text_bytes);

        Self {
            text,
            text_revision: 0,
            parsed,
            parse_error,
            parsed_at_text_revision: 0,
            text_dirty_for: None,
            source_format,
            layout,
            raw_bytes: bytes,
            validation: None,
        }
    }

    pub fn empty() -> Self {
        let text = TextDocument::empty();
        Self {
            text,
            text_revision: 0,
            parsed: Some(ParsedMeta {
                instance: or_io::meta::MetaInstance::new(""),
                revision: 0,
            }),
            parse_error: None,
            parsed_at_text_revision: 0,
            text_dirty_for: None,
            source_format: or_io::meta::SourceFormat::Xml,
            layout: None,
            raw_bytes: Vec::new(),
            validation: None,
        }
    }

    pub fn ensure_parsed_current(&mut self) {
        if self.parsed_at_text_revision == self.text_revision {
            return;
        }
        let xml: String = self.text.text.to_string();
        let result = or_io::meta::MetaDocument::parse(meta_schema(), xml.into_bytes());
        match result {
            Ok(parsed_doc) => {
                let prev_rev = self.parsed.as_ref().map(|p| p.revision).unwrap_or(0);
                self.parsed = Some(ParsedMeta {
                    instance: parsed_doc.instance,
                    revision: prev_rev.wrapping_add(1),
                });
                self.parse_error = None;
                // A binary body keeps its form: the XML typed in is its tree, saved in that form.
                if !parsed_doc.source_format.is_binary() && !self.source_format.is_binary() {
                    self.source_format = parsed_doc.source_format;
                }
            }
            Err(e) => {
                self.parse_error = Some(e.to_string());
            }
        }
        self.parsed_at_text_revision = self.text_revision;
    }

    pub fn after_visual_edit_in_place(&mut self) {
        if let Some(p) = self.parsed.as_mut() {
            p.revision = p.revision.wrapping_add(1);
        }
        self.parse_error = None;
        self.text_dirty_for = self.parsed.as_ref().map(|p| p.revision);
    }

    pub fn ensure_text_current(&mut self) {
        let Some(target_rev) = self.text_dirty_for else {
            return;
        };
        let Some(p) = self.parsed.as_ref() else {
            self.text_dirty_for = None;
            return;
        };
        if p.revision != target_rev {
            return;
        }
        self.regenerate_text_from_parsed();
        self.text_dirty_for = None;
    }

    fn regenerate_text_from_parsed(&mut self) {
        let xml = self
            .parsed
            .as_ref()
            .map(|p| or_io::meta::write_meta(&p.instance).unwrap_or_default())
            .unwrap_or_default();
        self.text = TextDocument::decode(xml.as_bytes());
        self.text_revision = self.text_revision.wrapping_add(1);
        self.parsed_at_text_revision = self.text_revision;
    }

    pub fn mark_text_edited(&mut self) {
        self.text_revision = self.text_revision.wrapping_add(1);
    }

    pub fn type_name(&self) -> &str {
        match &self.parsed {
            Some(p) => &p.instance.type_name,
            None => "",
        }
    }

    /// Why this document would not save, recomputed when its revision moves.
    ///
    /// Cached per revision, so the meta editor calls it every frame beside
    /// `ensure_parsed_current` without re-encoding.
    pub fn refresh_validation(&mut self) {
        let revision = self.revision();
        if matches!(self.validation, Some((at, _)) if at == revision) {
            return;
        }
        // ⚠ A headerless resource's save deflates it on a worker, so only its sections are written here.
        let refused = match self.headerless_sections() {
            Some(sections) => sections.err(),
            None => self.to_bytes().err(),
        };
        self.validation = Some((revision, refused));
    }

    /// Whether the tree or the text has changed since the document was read.
    pub fn is_edited(&self) -> bool {
        self.revision() != 0 || self.parsed_at_text_revision < self.text_revision
    }

    /// Whether this is a GTA VI PSO, whose `CHKS` checksum a save keeps as it was read.
    pub fn keeps_a_checksum(&self) -> bool {
        use or_io::meta::binary::ContainerLayout;
        matches!(self.layout.as_ref().map(|l| &l.container),
            Some(ContainerLayout::Pso(p)) if p.is_table_form() || p.is_gta6())
    }

    /// Whether this is a GTA VI body, a PSO of that game or a headerless resource, whose own
    /// table states its fields.
    pub fn is_gta_vi_body(&self) -> bool {
        use or_io::meta::binary::ContainerLayout;
        self.keeps_a_checksum()
            || matches!(self.layout.as_ref().map(|l| &l.container),
                Some(ContainerLayout::Rbm(rbm)) if rbm.seal.is_none())
    }

    /// A GTA VI headerless resource's sections with its tree written in; `None` for any other
    /// document.
    pub fn headerless_sections(
        &self,
    ) -> Option<Result<or_io::container::rpf::Rpf9Sections, String>> {
        use or_io::container::rpf::{Rpf9Form, Rpf9Sections};
        let Some(or_io::meta::binary::ContainerLayout::Rbm(rbm)) =
            self.layout.as_ref().map(|l| &l.container)
        else {
            return None;
        };
        if rbm.seal.is_some() {
            return None;
        }
        let written = self.instance_for_save().and_then(|instance| {
            or_io::meta::binary::rbm::gta6_grow_tree::write_grown(rbm, &instance)
                .map_err(|e| e.to_string())
        });
        Some(written.map(|(virtual_section, physical)| Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section,
            physical,
        }))
    }

    /// The refusal a save would produce, as the writer worded it.
    ///
    /// Not reworded: a second phrasing in the shell could drift from the
    /// refusal the save actually makes.
    pub fn why_it_will_not_save(&self) -> Option<&str> {
        match &self.validation {
            Some((_, Some(why))) => Some(why.as_str()),
            _ => None,
        }
    }

    pub fn source_format(&self) -> or_io::meta::SourceFormat {
        self.source_format
    }

    pub fn revision(&self) -> u64 {
        self.parsed.as_ref().map(|p| p.revision).unwrap_or(0)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        match self.source_format {
            or_io::meta::SourceFormat::Xml => Ok(self.text.encode()),
            or_io::meta::SourceFormat::Srl => {
                or_io::meta::srl::write(&self.instance_for_save()?).map_err(|e| e.to_string())
            }
            other => {
                let format = other
                    .to_binary()
                    .expect("non-XML SourceFormat must map to BinaryFormat");

                let instance = self.instance_for_save()?;
                or_io::meta::write_meta_binary(format, self.layout.as_ref(), &instance)
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn instance_for_save(&self) -> Result<or_io::meta::MetaInstance, String> {
        let text_is_newer = self.parsed_at_text_revision < self.text_revision;
        if text_is_newer {
            let xml: String = self.text.text.to_string();
            return or_io::meta::MetaDocument::parse(meta_schema(), xml.into_bytes())
                .map(|d| d.instance)
                .map_err(|e| format!("the edited XML does not parse, so it cannot be saved: {e}"));
        }
        self.parsed
            .as_ref()
            .map(|p| p.instance.clone())
            .ok_or_else(|| {
                format!(
                    "cannot save this {} document: it never parsed, so there is no value tree \
                     to encode. Fix the parse error, or use Save As to write XML.",
                    self.source_format.label()
                )
            })
    }
}
