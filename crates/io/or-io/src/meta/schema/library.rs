use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::types::*;

/// How a library spells a name it holds only as the literal hash of `name`: `hash_XXXXXXXX`.
pub fn hash_spelling(name: &str) -> String {
    format!("hash_{:08X}", crate::literal_string_hash(name.as_bytes()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStruct<'a> {
    pub def: &'a StructDef,
    pub chain: Vec<&'a StructDef>,
    pub fields: Vec<&'a Field>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaLibrary {
    pub structs: HashMap<TypeName, StructDef>,
    pub enums: HashMap<TypeName, EnumDef>,
}

impl SchemaLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_struct(&mut self, def: StructDef) {
        self.structs.insert(def.name.clone(), def);
    }

    pub fn insert_enum(&mut self, def: EnumDef) {
        self.enums.insert(def.name.clone(), def);
    }

    pub fn struct_(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }

    pub fn struct_any(&self, name: &str) -> Option<&StructDef> {
        if let Some(s) = self.struct_(name) {
            return Some(s);
        }
        let alt = super::normalize::convert_datatype_to_friendly_name(name);
        if alt == name {
            return None;
        }
        self.struct_(&alt)
    }

    pub fn enum_(&self, name: &str) -> Option<&EnumDef> {
        self.enums.get(name)
    }

    /// The structure `name` spells as written, in its friendly form, or as the spelling's hash.
    pub fn resolve_any(&self, name: &str) -> Option<ResolvedStruct<'_>> {
        if let Some(r) = self.resolve(name) {
            return Some(r);
        }
        let alt = super::normalize::convert_datatype_to_friendly_name(name);
        if alt != name {
            if let Some(r) = self.resolve(&alt) {
                return Some(r);
            }
        }
        self.resolve(&hash_spelling(name))
    }

    pub fn resolve(&self, name: &str) -> Option<ResolvedStruct<'_>> {
        let def = self.structs.get(name)?;
        let mut chain: Vec<&StructDef> = Vec::new();
        let mut cur = Some(def);
        let mut guard = 0usize;
        while let Some(d) = cur {
            chain.push(d);
            cur = d.base.as_deref().and_then(|b| self.structs.get(b));
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        chain.reverse();
        let fields = chain.iter().flat_map(|d| d.fields.iter()).collect();
        Some(ResolvedStruct { def, chain, fields })
    }

    pub fn descendants_of<'a>(&'a self, base: &'a str) -> Vec<&'a StructDef> {
        let mut out = Vec::new();
        for s in self.structs.values() {
            if self.is_descendant(&s.name, base) {
                out.push(s);
            }
        }
        out
    }

    fn is_descendant(&self, name: &str, base: &str) -> bool {
        if name == base {
            return false;
        }
        let mut cur = self.structs.get(name).and_then(|d| d.base.as_deref());
        let mut guard = 0usize;
        while let Some(b) = cur {
            if b == base {
                return true;
            }
            cur = self.structs.get(b).and_then(|d| d.base.as_deref());
            guard += 1;
            if guard > 64 {
                return false;
            }
        }
        false
    }

    pub fn len(&self) -> usize {
        self.structs.len() + self.enums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.structs.is_empty() && self.enums.is_empty()
    }

    pub fn apply_field_hints(&mut self) {
        for s in self.structs.values_mut() {
            for field in &mut s.fields {
                if matches!(field.hint, FieldHint::None) {
                    field.hint = crate::meta::field_hints::infer_hint(&field.name, &field.ty);
                }
            }
        }
    }
}
