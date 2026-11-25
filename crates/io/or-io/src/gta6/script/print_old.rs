//! Source constructs in the readable form as the four first passes always had it.
//!
//! ⚠ That form asks no pass and knows no declared field, no constructor and no operator of a
//! type: it writes a construct's keyword where its parts say what to write, and what it holds
//! otherwise. `reparse` reads this form back, so it stays as it is while true C# moves on.

use crate::gta6::script::ir::{BinOp, Callee, Expr, ExprSugar, Stmt};
use crate::gta6::script::tree;

use super::{assigned_receiver, bits, receiver, Work, Writer, POSTFIX, UNARY};

/// The last part of the name a call goes by: a method's own name.
fn called(callee: &Callee) -> &str {
    let name = match callee {
        Callee::Local { name, .. }
        | Callee::Import { name, .. }
        | Callee::Native { name, .. }
        | Callee::Intrinsic { name, .. } => name.as_str(),
        Callee::Builtin { path, .. } => path.as_str(),
        Callee::Virtual { method, .. } => method.as_str(),
    };
    name.rsplit('.').next().unwrap_or(name)
}

impl Writer<'_> {
    /// A source construct: its keyword and its blocks where its parts say what to write, else the statements it stands for.
    pub(super) fn old_sugar<'b>(
        &mut self,
        sugar: &'b tree::Sugar,
        indent: usize,
        work: &mut Vec<Work<'b>>,
    ) {
        match sugar {
            tree::Sugar::Hidden(_) => {}
            tree::Sugar::Group { parts, .. } => self.simples(parts, indent),
            tree::Sugar::Await {
                before,
                ready,
                after,
                ..
            } => {
                self.simples(before, indent);
                let awaited = self.value(receiver(ready).unwrap_or(ready));
                self.line(indent, &format!("await {awaited};"));
                self.simples(after, indent);
            }
            tree::Sugar::Yield { value, .. } => {
                self.simples(value, indent);
                self.line(indent, "yield return;");
            }
            tree::Sugar::Foreach {
                setup,
                cond,
                item,
                body,
                after,
                ..
            } => {
                // The collection is what the enumerator is taken from, else what each round asks.
                let (shown, source) = match setup.split_last() {
                    Some((taken, rest)) if assigned_receiver(taken).is_some() => {
                        (rest, assigned_receiver(taken))
                    }
                    _ => (setup.as_slice(), receiver(cond)),
                };
                let (Stmt::Assign { target, .. }, Some(source)) = (&**item, source) else {
                    return self.lowered(sugar, indent);
                };
                self.simples(shown, indent);
                let head = format!(
                    "foreach ({} {} in {})",
                    self.declared_type(*target),
                    self.var(*target),
                    self.value(source)
                );
                self.line(indent, &head);
                work.push(Work::Run(after, indent));
                self.open(body, indent, None, work);
            }
            tree::Sugar::Guarded { .. } | tree::Sugar::Try { .. } | tree::Sugar::Throw(_) => {
                self.blocked(sugar, indent, work);
            }
            tree::Sugar::SwitchOn { .. } => self.lowered(sugar, indent),
        }
    }

    /// `e` as the source construct `kind` reads it, where `e` has the shape that construct takes; else `e` as it is.
    pub(super) fn old_sugared(&mut self, kind: ExprSugar, e: &Expr) -> (String, u8) {
        match (kind, e) {
            (ExprSugar::Method, Expr::Call { callee, args, .. }) => {
                if let Some((receiver, passed)) = args.split_first() {
                    let receiver = self.expr(receiver, POSTFIX);
                    let text = format!("{receiver}.{}({})", called(callee), self.list(passed));
                    return (text, POSTFIX);
                }
            }
            (ExprSugar::Property, Expr::Call { callee, args, .. }) => {
                if let [receiver] = args.as_slice() {
                    let receiver = self.expr(receiver, POSTFIX);
                    return (format!("{receiver}.{}", called(callee)), POSTFIX);
                }
            }
            (ExprSugar::Field { owner }, Expr::Load { addr, offset, .. }) => {
                let base = self.expr(addr, POSTFIX);
                let text = match self.names.field(owner, *offset) {
                    Some(name) => format!("{base}.{name}"),
                    None => format!("{base}.f_{offset:X}"),
                };
                return (text, POSTFIX);
            }
            (ExprSugar::Lambda, Expr::FunctionRef { program, at }) => {
                return (self.function_ref(*program, *at), POSTFIX);
            }
            (ExprSugar::Coalesce, Expr::Select { then, els, .. }) => {
                let text = format!(
                    "{} ?? {}",
                    self.expr(bits(then), 2),
                    self.expr(bits(els), 2)
                );
                return (text, 1);
            }
            (ExprSugar::Concat, Expr::Call { args, .. }) if !args.is_empty() => {
                let p = BinOp::Add.precedence();
                let parts: Vec<String> = args.iter().map(|arg| self.expr(arg, p + 1)).collect();
                return (parts.join(" + "), p);
            }
            (ExprSugar::Out | ExprSugar::Ref, Expr::AddrOf { var, cell: 0 }) => {
                let word = match kind {
                    ExprSugar::Out => "out",
                    _ => "ref",
                };
                return (format!("{word} {}", self.var(*var)), UNARY);
            }
            (ExprSugar::Tuple, Expr::Cells(items)) => {
                return (format!("({})", self.list(items)), POSTFIX);
            }
            _ => {}
        }
        self.spelled(e)
    }
}
