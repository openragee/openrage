#![allow(clippy::write_with_newline)]

use std::fmt::Write;

use crate::meta::error::Result;

use super::value::{HashedString, MetaInstance, UnknownNode, Value};

pub fn write_meta(inst: &MetaInstance) -> Result<String> {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // An instance with no type and only kept elements is a document kept whole, root included.
    if inst.type_name.is_empty() && inst.fields.is_empty() && !inst.extras.is_empty() {
        inst.extras.iter().for_each(|e| write_extra(&mut out, e, 0));
    } else {
        write_struct(&mut out, inst, 0)?;
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_struct(out: &mut String, inst: &MetaInstance, depth: usize) -> Result<()> {
    write_indent(out, depth);
    if inst.fields.is_empty() && inst.extras.is_empty() {
        let _ = writeln!(out, "<{}/>", escape_tag(&inst.type_name));
        return Ok(());
    }
    let _ = writeln!(out, "<{}>", escape_tag(&inst.type_name));
    for (name, value) in &inst.fields {
        write_field(out, name, value, depth + 1)?;
    }
    for extra in &inst.extras {
        write_extra(out, extra, depth + 1);
    }
    write_indent(out, depth);
    let _ = writeln!(out, "</{}>", escape_tag(&inst.type_name));
    Ok(())
}

fn write_field(out: &mut String, name: &str, value: &Value, depth: usize) -> Result<()> {
    match value {
        Value::Null => Ok(()),
        Value::Bool(b) => {
            write_indent(out, depth);
            let _ = write!(out, "<{} value=\"{}\"/>\n", name, b);
            Ok(())
        }
        Value::Int(v) => emit_attr_value(out, name, &v.to_string(), depth),
        Value::UInt(v) => emit_attr_value(out, name, &v.to_string(), depth),
        Value::Float(v) => emit_attr_value(out, name, &format_float(*v), depth),
        Value::String(s) => emit_text(out, name, s, depth),
        Value::Hash(h) => emit_text(out, name, &h.to_xml_text(), depth),
        Value::Vec2([x, y]) => {
            write_indent(out, depth);
            let _ = write!(
                out,
                "<{name} x=\"{x}\" y=\"{y}\"/>\n",
                x = format_float(*x as f64),
                y = format_float(*y as f64)
            );
            Ok(())
        }
        Value::Vec3([x, y, z]) => {
            write_indent(out, depth);
            let _ = write!(
                out,
                "<{name} x=\"{x}\" y=\"{y}\" z=\"{z}\"/>\n",
                x = format_float(*x as f64),
                y = format_float(*y as f64),
                z = format_float(*z as f64)
            );
            Ok(())
        }
        Value::Vec4([x, y, z, w]) => {
            write_indent(out, depth);
            let _ = write!(
                out,
                "<{name} x=\"{x}\" y=\"{y}\" z=\"{z}\" w=\"{w}\"/>\n",
                x = format_float(*x as f64),
                y = format_float(*y as f64),
                z = format_float(*z as f64),
                w = format_float(*w as f64)
            );
            Ok(())
        }
        Value::Mat34(m) => {
            write_indent(out, depth);
            let _ = write!(out, "<{}>", name);
            for (i, row) in m.iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    if i + j > 0 {
                        out.push(' ');
                    }
                    let _ = write!(out, "{}", format_float(*v as f64));
                }
            }
            let _ = writeln!(out, "</{}>", name);
            Ok(())
        }
        Value::Color(c) => {
            let r = (*c >> 16) & 0xFF;
            let g = (*c >> 8) & 0xFF;
            let b = *c & 0xFF;
            let a = (*c >> 24) & 0xFF;
            write_indent(out, depth);
            let _ = write!(out, "<{name} r=\"{r}\" g=\"{g}\" b=\"{b}\" a=\"{a}\"/>\n");
            Ok(())
        }
        Value::Enum { variant, .. } => emit_text(out, name, variant, depth),
        Value::Bitset { bits, .. } => emit_text(out, name, &format!("0x{:X}", bits), depth),
        Value::Struct(inst) => {
            write_indent(out, depth);
            if inst.fields.is_empty() && inst.extras.is_empty() {
                let _ = writeln!(out, "<{}/>", name);
                return Ok(());
            }
            let _ = writeln!(out, "<{}>", name);
            for (n, v) in &inst.fields {
                write_field(out, n, v, depth + 1)?;
            }
            for extra in &inst.extras {
                write_extra(out, extra, depth + 1);
            }
            write_indent(out, depth);
            let _ = writeln!(out, "</{}>", name);
            Ok(())
        }
        Value::Pointer(opt) => match opt {
            None => {
                write_indent(out, depth);
                let _ = writeln!(out, "<{}/>", name);
                Ok(())
            }
            Some(inst) => {
                write_indent(out, depth);
                let _ = write!(out, "<{} type=\"{}\">\n", name, escape_tag(&inst.type_name));
                for (n, v) in &inst.fields {
                    write_field(out, n, v, depth + 1)?;
                }
                for extra in &inst.extras {
                    write_extra(out, extra, depth + 1);
                }
                write_indent(out, depth);
                let _ = writeln!(out, "</{}>", name);
                Ok(())
            }
        },
        Value::Array(items) => {
            if items.is_empty() {
                write_indent(out, depth);
                let _ = writeln!(out, "<{}/>", name);
                return Ok(());
            }
            write_indent(out, depth);
            let _ = writeln!(out, "<{}>", name);
            for item in items {
                write_field(out, "Item", item, depth + 1)?;
            }
            write_indent(out, depth);
            let _ = writeln!(out, "</{}>", name);
            Ok(())
        }
        Value::Map(pairs) => {
            if pairs.is_empty() {
                write_indent(out, depth);
                let _ = writeln!(out, "<{}/>", name);
                return Ok(());
            }
            write_indent(out, depth);
            let _ = writeln!(out, "<{}>", name);
            for (k, v) in pairs {
                write_field(out, "Key", k, depth + 1)?;
                write_field(out, "Value", v, depth + 1)?;
            }
            write_indent(out, depth);
            let _ = writeln!(out, "</{}>", name);
            Ok(())
        }
        Value::ByteArray(bytes) => {
            write_indent(out, depth);
            if bytes.is_empty() {
                let _ = writeln!(out, "<{name} content=\"char_array\"/>");
            } else {
                let _ = write!(out, "<{name} content=\"char_array\">");
                for (i, b) in bytes.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    let _ = write!(out, "{}", b);
                }
                let _ = writeln!(out, "</{name}>");
            }
            Ok(())
        }
        Value::Raw(s) => emit_text(out, name, s, depth),
    }
}

fn write_extra(out: &mut String, extra: &UnknownNode, depth: usize) {
    write_indent(out, depth);
    let _ = write!(out, "<{}", escape_tag(&extra.tag));
    for (k, v) in &extra.attrs {
        let _ = write!(out, " {}=\"{}\"", k, escape_attr(v));
    }
    if !extra.children.is_empty() {
        let _ = writeln!(out, ">");
        for child in &extra.children {
            write_extra(out, child, depth + 1);
        }
        write_indent(out, depth);
        let _ = writeln!(out, "</{}>", escape_tag(&extra.tag));
    } else if extra.text.trim().is_empty() {
        let _ = writeln!(out, "/>");
    } else {
        let _ = write!(
            out,
            ">{}</{}>\n",
            escape_text(&extra.text),
            escape_tag(&extra.tag)
        );
    }
}

fn emit_text(out: &mut String, name: &str, text: &str, depth: usize) -> Result<()> {
    write_indent(out, depth);
    if text.is_empty() {
        let _ = writeln!(out, "<{}/>", name);
    } else {
        let _ = writeln!(out, "<{}>{}</{}>", name, escape_text(text), name);
    }
    Ok(())
}

fn emit_attr_value(out: &mut String, name: &str, value: &str, depth: usize) -> Result<()> {
    write_indent(out, depth);
    let _ = write!(out, "<{} value=\"{}\"/>\n", name, escape_attr(value));
    Ok(())
}

pub(crate) fn format_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{:.1}", v)
    } else {
        let mut s = format!("{}", v);
        if !s.contains('.') && !s.contains('e') && !s.contains('E') {
            s.push_str(".0");
        }
        s
    }
}

/// ⚠ A type name is written in its `A__B` spelling, never `A::B`.
///
/// A colon is XML's namespace separator, so `<A::B>` parses as element `B` in
/// namespace `A:` and the type resolves to whatever else is called `B`, or to
/// nothing. Every nested PSC type is spelled with `::`; with a schema snapshot
/// that spelling lost 8 of the 15 documents the XML leg failed on.
/// `resolve_any` converts back.
fn escape_tag(s: &str) -> String {
    crate::meta::schema::consistent_struct_short_name(&s.replace(' ', "_"))
}

pub(crate) fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const _: fn(&HashedString) = |_| {};
