use std::sync::Arc;

use egui_phosphor::regular as ph;
use or_io::meta::{
    ArrayKind, BitsetStorage, EnumDef, Field, FieldHint, FieldType, MetaInstance, PointerPolicy,
    StringKind, Value,
};

use crate::state::name_index::NameIndexSnapshot;
use crate::state::reference_names::Reference;
use crate::state::MetaEdit;
use crate::ui::panels::drag_value;
use crate::ui::theme;

use super::ctx::EditorCtx;

pub fn draw_struct(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    fields: &[Field],
    inst: &mut MetaInstance,
    dirty: &mut bool,
) {
    let ctx = &EditorCtx {
        references: ctx.references.within(inst),
        ..ctx.clone()
    };
    egui::Grid::new(("meta_struct_grid", inst.type_name.as_str()))
        .num_columns(2)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            // A document read without a schema may repeat a name, as a root `Item` array does.
            let mut seen: std::collections::HashMap<&str, usize> = Default::default();
            for field in fields {
                let nth = seen.entry(field.name.as_str()).or_default();
                draw_field_row(ui, ctx, field, *nth, inst, dirty);
                *nth += 1;
            }
        });

    if !inst.extras.is_empty() {
        ui.add_space(theme::SPACING_DEFAULT);
        ui.collapsing(
            egui::RichText::new(format!("Unknown elements ({})", inst.extras.len()))
                .small()
                .color(theme::TEXT_DIM),
            |ui| {
                for extra in &inst.extras {
                    ui.label(format!("<{}> {}", extra.tag, extra.text));
                }
            },
        );
    }
}

fn draw_field_row(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    nth: usize,
    inst: &mut MetaInstance,
    dirty: &mut bool,
) {
    let found = inst
        .fields
        .iter()
        .enumerate()
        .filter(|(_, (n, _))| n == &field.name)
        .nth(nth)
        .map(|(i, _)| i);

    let label = field_label(field);
    ui.label(label);

    // ⚠ A grid's cells share one ID scope, so without the field's name every
    // array's element `[0]` is the same widget to egui.
    ui.push_id((field.name.as_str(), nth), |ui| match found {
        Some(index) => {
            let value = &mut inst.fields[index].1;
            draw_field(ui, &ctx.field(&field.name, nth), field, value, dirty);
        }
        None => draw_absent(ui, ctx, field, inst, dirty),
    });
    ui.end_row();
}

/// A field the file does not set, which gets its default only from the user's Add.
fn draw_absent(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    inst: &mut MetaInstance,
    dirty: &mut bool,
) {
    let add = ui
        .small_button(egui::RichText::new(format!("{} Add default", ph::PLUS)))
        .on_hover_text("This file does not set the field. Add it with its default value.");
    if add.clicked() {
        let (path, name, at) = (ctx.path.clone(), field.name.clone(), inst.fields.len());
        inst.fields.push((name.clone(), default_value(&field.ty)));
        ctx.record(MetaEdit::Field {
            path,
            name,
            at,
            other: None,
        });
        *dirty = true;
    }
}

/// A value of another shape than the schema states, left as the file holds it.
fn not_this_shape(ui: &mut egui::Ui, expected: &str) {
    ui.label(
        egui::RichText::new(format!("(the file holds no {expected} here)"))
            .small()
            .color(theme::TEXT_DIM),
    );
}

pub fn draw_field(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    value: &mut Value,
    dirty: &mut bool,
) {
    // ⚠ A leaf is replaced whole, so the value it held is its step; a struct,
    // pointer or array records its own edits, and is not copied every frame.
    let before = is_leaf(&field.ty).then(|| value.clone());
    let mut changed = false;
    draw_value(ui, ctx, field, value, &mut changed);
    if changed {
        *dirty = true;
        if let Some(before) = before {
            ctx.changed(before);
        }
    }
}

fn is_leaf(ty: &FieldType) -> bool {
    !matches!(
        ty,
        FieldType::Struct(_)
            | FieldType::Pointer { .. }
            | FieldType::Array { .. }
            | FieldType::Map { .. }
    )
}

fn draw_value(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    value: &mut Value,
    dirty: &mut bool,
) {
    // ⚠ An array's elements carry its name, and only an element holds an index.
    if !matches!(field.ty, FieldType::Array { .. }) {
        let index = match ctx.references.for_field(&field.name) {
            Some(Reference::Room(rooms)) => Some((rooms, "room")),
            Some(Reference::Entity(entities)) => Some((entities, "entity")),
            _ => None,
        };
        if let Some((names, noun)) = index {
            super::reference::draw_index(ui, field, names, noun, value, dirty);
            return;
        }
    }
    match &field.ty {
        FieldType::Bool => draw_bool(ui, value, dirty),
        FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
            draw_uint(ui, field, value, dirty);
            super::text_names::draw(ui, ctx, value);
        }
        FieldType::S8 | FieldType::S16 | FieldType::S32 | FieldType::S64 => {
            draw_int(ui, field, value, dirty);
        }
        FieldType::Float | FieldType::Float16 | FieldType::Double => {
            draw_float(ui, field, value, dirty);
        }
        FieldType::String(kind) => draw_string(ui, ctx, field, *kind, value, dirty),
        FieldType::Vector2 | FieldType::Vec2V => draw_vec(ui, value, 2, dirty),
        FieldType::Vector3 | FieldType::Vec3V => draw_vec(ui, value, 3, dirty),
        FieldType::Vector4 | FieldType::Vec4V => draw_vec(ui, value, 4, dirty),
        FieldType::Color32 => draw_color(ui, value, dirty),
        FieldType::Enum(name) => draw_enum(ui, ctx, name, value, dirty),
        FieldType::Bitset { values, storage } => {
            draw_bitset(ui, ctx, values, *storage, value, dirty)
        }
        FieldType::Struct(name) => draw_nested_struct(ui, ctx, name, value, dirty),
        FieldType::Pointer { base, policy } => draw_pointer(ui, ctx, base, *policy, value, dirty),
        FieldType::Array { elem, kind } => {
            draw_array(ui, ctx, field, elem, kind.clone(), value, dirty)
        }
        FieldType::Map { key, value: of } => {
            super::map::draw_map(ui, ctx, field, (&**key, &**of), value, dirty)
        }
        FieldType::Pad(_) => {
            ui.label(
                egui::RichText::new("(padding)")
                    .small()
                    .color(theme::TEXT_DIM),
            );
        }
        FieldType::Mat34V
        | FieldType::Matrix34
        | FieldType::Mat33V
        | FieldType::Mat44V
        | FieldType::Matrix33
        | FieldType::Matrix44 => {
            ui.label(
                egui::RichText::new("(matrix — edit via XML source)")
                    .small()
                    .color(theme::TEXT_DIM),
            );
        }
        FieldType::Unknown(tag) if tag == super::bytes::TAG => super::bytes::draw(ui, value, dirty),
        FieldType::Unknown(tag) => {
            ui.label(
                egui::RichText::new(format!("(unknown: {tag})"))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        }
    }
}

fn draw_bool(ui: &mut egui::Ui, value: &mut Value, dirty: &mut bool) {
    let mut b = match value {
        Value::Bool(b) => *b,
        _ => false,
    };
    if ui.checkbox(&mut b, "").changed() {
        *value = Value::Bool(b);
        *dirty = true;
    }
}

/// A number of `field`, named after it for a screen reader; whether it was changed.
fn number<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    field: &Field,
    value: &mut N,
    style: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>,
) -> bool {
    let drawn = drag_value(ui, value, style);
    crate::ui::accessible_name::named(drawn.response, &field.name);
    drawn.inner
}

fn draw_uint(ui: &mut egui::Ui, field: &Field, value: &mut Value, dirty: &mut bool) {
    let mut v: u64 = match value {
        Value::UInt(v) => *v,
        Value::Int(v) => (*v).max(0) as u64,
        _ => 0,
    };
    let step = field.step.unwrap_or(1.0);
    let range = match (field.min, field.max) {
        (Some(min), Some(max)) => min.max(0.0) as u64..=max.max(0.0) as u64,
        _ => u64::MIN..=u64::MAX,
    };
    if number(ui, field, &mut v, |d| d.speed(step).range(range)) {
        *value = Value::UInt(v);
        *dirty = true;
    }
}

fn draw_int(ui: &mut egui::Ui, field: &Field, value: &mut Value, dirty: &mut bool) {
    let mut v: i64 = match value {
        Value::Int(v) => *v,
        Value::UInt(v) => *v as i64,
        _ => 0,
    };
    let step = field.step.unwrap_or(1.0);
    let range = match (field.min, field.max) {
        (Some(min), Some(max)) => min as i64..=max as i64,
        _ => i64::MIN..=i64::MAX,
    };
    if number(ui, field, &mut v, |d| d.speed(step).range(range)) {
        *value = Value::Int(v);
        *dirty = true;
    }
}

fn draw_float(ui: &mut egui::Ui, field: &Field, value: &mut Value, dirty: &mut bool) {
    let mut v: f64 = match value {
        Value::Float(v) => *v,
        Value::Int(v) => *v as f64,
        _ => 0.0,
    };
    let speed = field.step.unwrap_or(0.05);
    let range = match (field.min, field.max) {
        (Some(min), Some(max)) => min..=max,
        _ => f64::NEG_INFINITY..=f64::INFINITY,
    };
    if number(ui, field, &mut v, |d| {
        d.speed(speed).max_decimals(6).range(range)
    }) {
        *value = Value::Float(v);
        *dirty = true;
    }
}

fn draw_string(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    kind: StringKind,
    value: &mut Value,
    dirty: &mut bool,
) {
    let canonical: String = match value {
        Value::String(s) => s.clone(),
        Value::Hash(h) if h.raw.is_empty() => h.to_xml_text(),
        Value::Hash(h) => h.raw.clone(),
        _ => String::new(),
    };
    let buf_id = ui.id().with(("meta_string_buf", field.name.as_str()));
    let mut buf: String = ui
        .data(|d| d.get_temp::<String>(buf_id))
        .unwrap_or_else(|| canonical.clone());

    let response = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .desired_width(220.0)
            .font(egui::TextStyle::Monospace),
    );

    let focused = response.has_focus() || response.gained_focus();
    if !focused && buf != canonical {
        buf = canonical.clone();
    }
    ui.data_mut(|d| d.insert_temp::<String>(buf_id, buf.clone()));

    if response.changed() {
        *value = match kind {
            StringKind::Plain | StringKind::AtString => Value::String(buf.clone()),
            _ => Value::Hash(or_io::meta::instance::HashedString::from_xml_text(&buf)),
        };
        *dirty = true;
    }

    if matches!(
        kind,
        StringKind::AtHashString | StringKind::AtFinalHashString | StringKind::AtHashValue
    ) {
        if let Value::Hash(h) = value {
            ui.label(
                egui::RichText::new(format!("0x{:08X}", h.hash))
                    .small()
                    .monospace()
                    .color(theme::TEXT_DIM),
            )
            .on_hover_text("Jenkins-1 hash (RAGE atHashString)");
        }
    }
    super::text_names::draw(ui, ctx, value);

    if let Some(category) = super::picker::picker_category(field.hint) {
        if ui
            .small_button(egui::RichText::new(ph::DOTS_THREE_OUTLINE))
            .on_hover_text("Pick from indexed game assets")
            .clicked()
        {
            super::picker::open_picker(ui.ctx(), category);
        }
        if let Some(picked) = super::picker::take_picker_choice(ui.ctx(), category) {
            *value = match kind {
                StringKind::Plain | StringKind::AtString => Value::String(picked),
                _ => Value::Hash(or_io::meta::instance::HashedString::new(picked)),
            };
            *dirty = true;
        }
        super::picker::draw_picker_modal(ui, ctx, category);
    }

    let choice = match ctx.references.for_field(&field.name) {
        Some(Reference::Archetype(names)) => (names, "Pick an archetype this document declares"),
        Some(Reference::EntitySet(names)) => (names, "Pick an entity set the interior declares"),
        Some(Reference::ScenarioType(names)) => {
            (names, "Pick a scenario type the install declares")
        }
        Some(Reference::ModelSet(names)) => (names, "Pick a model set the install declares"),
        Some(Reference::ProceduralTag(names)) => {
            (names, "Pick a procedural tag the install declares")
        }
        _ => return,
    };
    super::reference::draw_name_choice(ui, choice.0, choice.1, kind, value, dirty);
}

fn draw_vec(ui: &mut egui::Ui, value: &mut Value, dim: usize, dirty: &mut bool) {
    let mut buf = match value {
        Value::Vec2(v) => [v[0], v[1], 0.0, 0.0],
        Value::Vec3(v) => [v[0], v[1], v[2], 0.0],
        Value::Vec4(v) => [v[0], v[1], v[2], v[3]],
        _ => [0.0; 4],
    };
    let mut edited = false;
    ui.horizontal(|ui| {
        for (i, label) in ["x", "y", "z", "w"].iter().enumerate().take(dim) {
            ui.label(egui::RichText::new(*label).small().color(theme::TEXT_DIM));
            edited |= drag_value(ui, &mut buf[i], |d| d.speed(0.05).max_decimals(4)).inner;
        }
    });
    if !edited {
        return;
    }
    *dirty = true;
    *value = match dim {
        2 => Value::Vec2([buf[0], buf[1]]),
        3 => Value::Vec3([buf[0], buf[1], buf[2]]),
        4 => Value::Vec4(buf),
        _ => Value::Null,
    };
}

fn draw_color(ui: &mut egui::Ui, value: &mut Value, dirty: &mut bool) {
    let mut packed = match value {
        Value::Color(c) => *c,
        _ => 0xFFFFFFFF,
    };
    let mut rgba = [
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
        ((packed >> 24) & 0xFF) as u8,
    ];
    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
        packed = ((rgba[3] as u32) << 24)
            | ((rgba[0] as u32) << 16)
            | ((rgba[1] as u32) << 8)
            | (rgba[2] as u32);
        *value = Value::Color(packed);
        *dirty = true;
    }
}

fn draw_enum(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    type_name: &str,
    value: &mut Value,
    dirty: &mut bool,
) {
    let Some(enum_def) = ctx.library.enum_(type_name) else {
        ui.colored_label(theme::TEXT_DIM, format!("(unknown enum {type_name})"));
        return;
    };
    let mut current = match value {
        Value::Enum { variant, .. } => variant.clone(),
        _ => enum_def
            .values
            .first()
            .map(|v| v.name.clone())
            .unwrap_or_default(),
    };
    let display = current.clone();
    let mut new_choice: Option<String> = None;
    egui::ComboBox::from_id_salt(("meta_enum", type_name))
        .selected_text(display)
        .width(220.0)
        .show_ui(ui, |ui| {
            for v in &enum_def.values {
                let resp = ui.selectable_label(v.name == current, &v.name);
                if let Some(desc) = &v.description {
                    resp.clone().on_hover_text(desc);
                }
                if resp.clicked() {
                    new_choice = Some(v.name.clone());
                }
            }
        });
    if let Some(c) = new_choice {
        current = c;
        *value = Value::Enum {
            type_name: type_name.to_string(),
            variant: current,
        };
        *dirty = true;
    }
}

fn draw_bitset(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    type_name: &str,
    _storage: BitsetStorage,
    value: &mut Value,
    dirty: &mut bool,
) {
    let Some(enum_def) = ctx.library.enum_(type_name) else {
        ui.colored_label(theme::TEXT_DIM, format!("(unknown bitset {type_name})"));
        return;
    };
    let mut bits = match value {
        Value::Bitset { bits, .. } => *bits,
        _ => 0u128,
    };
    let count = bit_count(enum_def, bits);
    let label = if count == 0 {
        "(none)".to_string()
    } else {
        format!("{count} selected")
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(("meta_bitset", type_name))
        .selected_text(label)
        .width(220.0)
        .show_ui(ui, |ui| {
            for (idx, v) in enum_def.values.iter().enumerate() {
                let bit = bit_for(enum_def, idx);
                let mut on = bits & bit != 0;
                if ui.checkbox(&mut on, &v.name).changed() {
                    if on {
                        bits |= bit;
                    } else {
                        bits &= !bit;
                    }
                    changed = true;
                }
            }
        });
    if changed {
        *value = Value::Bitset {
            type_name: type_name.to_string(),
            bits,
        };
        *dirty = true;
    }
}

fn bit_for(enum_def: &EnumDef, idx: usize) -> u128 {
    let v = enum_def.variant_value(idx);
    if (0..128).contains(&v) {
        1u128 << v
    } else {
        0
    }
}

fn bit_count(enum_def: &EnumDef, bits: u128) -> u32 {
    let mut c = 0u32;
    for (i, _) in enum_def.values.iter().enumerate() {
        if bits & bit_for(enum_def, i) != 0 {
            c += 1;
        }
    }
    c
}

fn draw_nested_struct(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    type_name: &str,
    value: &mut Value,
    dirty: &mut bool,
) {
    let Value::Struct(inner) = value else {
        not_this_shape(ui, type_name);
        return;
    };
    let shown_type = if inner.type_name.is_empty() {
        type_name.to_string()
    } else {
        inner.type_name.clone()
    };
    let resolved = ctx.library.resolve(&shown_type);
    let id = ui.make_persistent_id(("meta_nested", type_name, &shown_type));
    egui::CollapsingHeader::new(egui::RichText::new(&shown_type).monospace())
        .id_salt(id)
        .default_open(false)
        .show(ui, |ui| {
            let fields: Vec<Field> = match resolved {
                Some(r) => r.fields.iter().map(|f| (*f).clone()).collect(),
                None => super::derived::fields_of(inner),
            };
            draw_struct(ui, ctx, &fields, inner, dirty);
        });
}

fn draw_pointer(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    base: &str,
    _policy: PointerPolicy,
    value: &mut Value,
    dirty: &mut bool,
) {
    let stash_id = ui.id().with(("meta_pointer_stash", base));
    let stash_present: bool = ui.ctx().data(|d| {
        d.get_temp::<Option<MetaInstance>>(stash_id)
            .flatten()
            .is_some()
    });

    let mut request_remove = false;
    let mut request_restore = false;
    let mut request_add = false;

    ui.horizontal(|ui| match value {
        Value::Pointer(Some(inner)) => {
            let descendants = ctx.library.descendants_of(base);
            if descendants.is_empty() {
                ui.label(
                    egui::RichText::new(&inner.type_name)
                        .monospace()
                        .color(theme::TEXT_PRIMARY),
                );
            } else {
                egui::ComboBox::from_id_salt(("meta_ptr_type", base))
                    .selected_text(inner.type_name.clone())
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(inner.type_name == base, base).clicked() {
                            ctx.changed(Value::Pointer(Some(inner.clone())));
                            inner.type_name = base.to_string();
                            *dirty = true;
                        }
                        for d in descendants {
                            if ui
                                .selectable_label(inner.type_name == d.name, &d.name)
                                .clicked()
                            {
                                ctx.changed(Value::Pointer(Some(inner.clone())));
                                inner.type_name = d.name.clone();
                                *dirty = true;
                            }
                        }
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(egui::RichText::new(format!("{} Remove", ph::TRASH)))
                    .on_hover_text(
                        "Set this pointer to null. The current contents are kept \
                             aside so you can Restore them with one click.",
                    )
                    .clicked()
                {
                    request_remove = true;
                }
            });
        }
        _ => {
            ui.label(egui::RichText::new("(null)").small().color(theme::TEXT_DIM));
            if ui
                .small_button(egui::RichText::new(format!("{} Add {}", ph::PLUS, base)))
                .on_hover_text("Create an empty payload for this pointer.")
                .clicked()
            {
                request_add = true;
            }
            if stash_present
                && ui
                    .small_button(egui::RichText::new(format!(
                        "{} Restore last value",
                        ph::ARROW_COUNTER_CLOCKWISE
                    )))
                    .on_hover_text(
                        "Put the most recently removed payload back. \
                             Discarded after the next visual edit lands.",
                    )
                    .clicked()
            {
                request_restore = true;
            }
        }
    });

    if request_remove {
        let prev = std::mem::replace(value, Value::Pointer(None));
        ctx.changed(prev.clone());
        if let Value::Pointer(Some(inner)) = prev {
            ui.ctx()
                .data_mut(|d| d.insert_temp::<Option<MetaInstance>>(stash_id, Some(*inner)));
        }
        *dirty = true;
    } else if request_restore {
        if let Some(inst) = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<Option<MetaInstance>>(stash_id))
            .flatten()
        {
            ctx.changed(std::mem::replace(
                value,
                Value::Pointer(Some(Box::new(inst))),
            ));
            *dirty = true;
        }
    } else if request_add {
        let added = Value::Pointer(Some(Box::new(MetaInstance::new(base))));
        ctx.changed(std::mem::replace(value, added));
        ui.ctx().data_mut(|d| {
            d.remove_temp::<Option<MetaInstance>>(stash_id);
        });
        *dirty = true;
    }

    if let Value::Pointer(Some(inner)) = value {
        let fields: Vec<Field> = match ctx.library.resolve(&inner.type_name) {
            Some(r) => r.fields.iter().map(|f| (*f).clone()).collect(),
            None => super::derived::fields_of(inner),
        };
        ui.vertical(|ui| {
            ui.indent(("ptr_indent", base), |ui| {
                draw_struct(ui, ctx, &fields, inner, dirty);
            });
        });
    }
}

fn draw_array(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    array: &Field,
    elem: &FieldType,
    _kind: ArrayKind,
    value: &mut Value,
    dirty: &mut bool,
) {
    let Value::Array(items) = value else {
        not_this_shape(ui, "array");
        return;
    };

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} item(s)", items.len()))
                    .small()
                    .color(theme::TEXT_DIM),
            );
            if ui
                .small_button(egui::RichText::new(format!("{} Add", ph::PLUS)))
                .clicked()
            {
                ctx.record(element(ctx, items.len(), None));
                items.push(default_value(elem));
                *dirty = true;
            }
            super::presets::array_header(ui, ctx, items);
        });
        let mut to_delete: Option<usize> = None;
        let mut to_move: Option<(usize, isize)> = None;
        let total = items.len();
        for (idx, item) in items.iter_mut().enumerate() {
            ui.push_id(("meta_array_item", idx), |ui| {
                let summary = item_summary(item);
                let header_id = ui.make_persistent_id(("meta_array_hdr", idx));
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    header_id,
                    false,
                );
                let header_resp = ui.horizontal(|ui| {
                    state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                    super::presets::item_controls(ui, ctx, idx, item);
                    ui.label(
                        egui::RichText::new(format!("[{idx}]"))
                            .monospace()
                            .color(theme::TEXT_DIM),
                    );
                    if !summary.is_empty() {
                        ui.label(egui::RichText::new(&summary).monospace());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(egui::RichText::new(ph::TRASH))
                            .on_hover_text("Delete this item")
                            .clicked()
                        {
                            to_delete = Some(idx);
                        }
                        if idx + 1 < total
                            && ui
                                .small_button(egui::RichText::new(ph::ARROW_DOWN))
                                .on_hover_text("Move down")
                                .clicked()
                        {
                            to_move = Some((idx, 1));
                        }
                        if idx > 0
                            && ui
                                .small_button(egui::RichText::new(ph::ARROW_UP))
                                .on_hover_text("Move up")
                                .clicked()
                        {
                            to_move = Some((idx, -1));
                        }
                    });
                });
                state.show_body_indented(&header_resp.response, ui, |ui| {
                    draw_field(ui, &ctx.index(idx), &element_of(array, elem), item, dirty);
                });
            });
        }
        super::presets::forget_selection_if(ctx, to_delete.is_some() || to_move.is_some());
        if let Some(i) = to_delete {
            ctx.record(element(ctx, i, Some(items.remove(i))));
            *dirty = true;
        }
        if let Some((i, delta)) = to_move {
            let new = i as isize + delta;
            if new >= 0 && (new as usize) < items.len() {
                items.swap(i, new as usize);
                let (path, a, b) = (ctx.path.clone(), i, new as usize);
                ctx.record(MetaEdit::Swap { path, a, b });
                *dirty = true;
            }
        }
    });
}

/// Element `at` of the array `ctx` draws, held as `other` while out of it.
fn element(ctx: &EditorCtx, at: usize, other: Option<Value>) -> MetaEdit {
    MetaEdit::Element {
        path: ctx.path.clone(),
        at,
        other,
    }
}

/// An array's element, named after the array so a reference lookup still finds it.
fn element_of(array: &Field, elem: &FieldType) -> Field {
    Field {
        name: array.name.clone(),
        ty: elem.clone(),
        init: None,
        min: None,
        max: None,
        step: None,
        ui_key: false,
        hide_widgets: false,
        description: None,
        hint: FieldHint::None,
    }
}

pub(crate) fn item_summary(value: &Value) -> String {
    use or_io::meta::Value as V;
    let inst = match value {
        V::Struct(b) => Some(b.as_ref()),
        V::Pointer(Some(b)) => Some(b.as_ref()),
        _ => None,
    };
    let Some(inst) = inst else {
        return String::new();
    };
    let mut prefix = String::new();
    if !inst.type_name.is_empty() {
        prefix.push_str(&inst.type_name);
    }
    for (n, v) in &inst.fields {
        match v {
            V::Hash(h) if !h.raw.is_empty() => {
                return if prefix.is_empty() {
                    format!("{}={}", n, h.raw)
                } else {
                    format!("{}  ({}={})", prefix, n, h.raw)
                };
            }
            V::String(s) if !s.is_empty() => {
                return if prefix.is_empty() {
                    format!("{}={}", n, s)
                } else {
                    format!("{}  ({}={})", prefix, n, s)
                };
            }
            _ => {}
        }
    }
    prefix
}

fn field_label(field: &Field) -> egui::WidgetText {
    let mut text = egui::RichText::new(&field.name).monospace();
    if field.ui_key {
        text = text.color(theme::ACCENT_TEXT);
    }
    let mut wt: egui::WidgetText = text.into();
    if let Some(desc) = &field.description {
        wt = egui::WidgetText::from(field.name.clone());
        let _ = desc;
    }
    wt
}

pub(super) fn default_value(ty: &FieldType) -> Value {
    match ty {
        FieldType::Bool => Value::Bool(false),
        FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => Value::UInt(0),
        FieldType::S8 | FieldType::S16 | FieldType::S32 | FieldType::S64 => Value::Int(0),
        FieldType::Float | FieldType::Float16 | FieldType::Double => Value::Float(0.0),
        FieldType::String(StringKind::Plain) | FieldType::String(StringKind::AtString) => {
            Value::String(String::new())
        }
        FieldType::String(_) => Value::Hash(or_io::meta::instance::HashedString::default()),
        FieldType::Vector2 | FieldType::Vec2V => Value::Vec2([0.0; 2]),
        FieldType::Vector3 | FieldType::Vec3V => Value::Vec3([0.0; 3]),
        FieldType::Vector4 | FieldType::Vec4V => Value::Vec4([0.0; 4]),
        FieldType::Color32 => Value::Color(0xFFFFFFFF),
        FieldType::Enum(name) => Value::Enum {
            type_name: name.clone(),
            variant: String::new(),
        },
        FieldType::Bitset { values, .. } => Value::Bitset {
            type_name: values.clone(),
            bits: 0,
        },
        FieldType::Struct(name) => Value::Struct(Box::new(MetaInstance::new(name.clone()))),
        FieldType::Pointer { .. } => Value::Pointer(None),
        FieldType::Array { .. } => Value::Array(Vec::new()),
        FieldType::Map { .. } => Value::Map(Vec::new()),
        FieldType::Unknown(tag) if tag == super::bytes::TAG => Value::ByteArray(Vec::new()),
        _ => Value::Null,
    }
}

const _: fn(Arc<NameIndexSnapshot>) = |_| {};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::reference_names::ReferenceNames;

    fn field(name: &str, ty: FieldType, range: Option<(f64, f64)>) -> Field {
        Field {
            name: name.to_string(),
            ty,
            init: None,
            min: range.map(|(lo, _)| lo),
            max: range.map(|(_, hi)| hi),
            step: None,
            ui_key: false,
            hide_widgets: false,
            description: None,
            hint: FieldHint::None,
        }
    }

    /// Nothing the file leaves out is filled in, and nothing it holds is
    /// clamped, converted or replaced, by drawing it.
    #[test]
    fn drawing_the_form_leaves_the_instance_as_the_file_holds_it() {
        let fields = [
            field("absent", FieldType::U32, None),
            field("beyond", FieldType::U8, Some((0.0, 10.0))),
            field("unset", FieldType::Float, None),
            field("wide", FieldType::Vector3, None),
            field("nested", FieldType::Struct("CThing".to_string()), None),
            field(
                "list",
                FieldType::Array {
                    elem: Box::new(FieldType::U8),
                    kind: ArrayKind::AtArray,
                },
                None,
            ),
        ];
        let mut inst = MetaInstance::new("T");
        inst.set("beyond", Value::UInt(200));
        inst.set("unset", Value::Float(f64::NAN));
        inst.set("wide", Value::Vec4([1.0, 2.0, 3.0, 4.0]));
        inst.set("nested", Value::Null);
        inst.set("list", Value::Null);
        // ⚠ Compared as text, because a NaN is not equal to itself.
        let before = format!("{inst:?}");

        let ctx = EditorCtx::for_test();
        let egui_ctx = egui::Context::default();
        let mut dirty = false;
        for _ in 0..2 {
            let _ = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_struct(ui, &ctx, &fields, &mut inst, &mut dirty);
            });
        }
        assert!(!dirty, "drawing reported an edit");
        assert_eq!(format!("{inst:?}"), before, "drawing changed the instance");
    }

    #[test]
    fn an_array_element_is_asked_about_under_the_arrays_name() {
        let mut room = MetaInstance::new("CMloRoomDef");
        room.set("name", Value::String("hall".to_string()));
        let mut interior = MetaInstance::new("CMloArchetypeDef");
        interior.set("rooms", Value::Array(vec![Value::Struct(Box::new(room))]));
        let mut set = MetaInstance::new("CMloEntitySet");
        set.set("locations", Value::Array(vec![Value::UInt(0)]));
        let references = ReferenceNames::default().within(&interior).within(&set);

        let fields = super::super::derived::fields_of(&set);
        let locations = fields
            .iter()
            .find(|f| f.name == "locations")
            .expect("the set's locations");
        let FieldType::Array { elem, .. } = &locations.ty else {
            panic!("locations is an array: {:?}", locations.ty);
        };
        let rooms = ["hall".to_string()];
        assert_eq!(
            references.for_field(&element_of(locations, elem).name),
            Some(Reference::Room(&rooms[..]))
        );
    }
}
