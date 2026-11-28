//! A function written as the member of a class its role makes it: a constructor, the accessor of
//! a property, a method with what is proved of its slot.
//!
//! ⚠ A role is taken only where the function has the shape the role's form needs: a getter
//! takes nothing and yields a value, a setter takes one value and yields none. Any other
//! function is written as a method, whatever its role says.

use crate::gta6::script::class::{Method, Role};
use crate::gta6::script::ir::{Expr, FunctionKey, Kind, Returns, TypeKey, Var};
use crate::gta6::script::passes::{async_await, classes};
use crate::gta6::script::tree;
use crate::gta6::script::Pass;

use super::{placement, Writer};

/// Whether every `return` of a value in `body` hands back `back`: a method of a task then returns nothing else.
fn only_hands_back(body: &[tree::Stmt], back: &Expr) -> bool {
    // An explicit stack: a body nests as deep as its listing does.
    let mut open: Vec<&[tree::Stmt]> = vec![body];
    while let Some(list) = open.pop() {
        for stmt in list {
            match stmt {
                tree::Stmt::Return(Some(value)) if value != back => return false,
                tree::Stmt::If { then, els, .. } => open.extend([then.as_slice(), els.as_slice()]),
                tree::Stmt::While { body, .. }
                | tree::Stmt::DoWhile { body, .. }
                | tree::Stmt::For { body, .. } => open.push(body),
                tree::Stmt::Switch { cases, .. } => {
                    open.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                tree::Stmt::Sugar(sugar) => open.extend(sugar.parts().bodies),
                _ => {}
            }
        }
    }
    true
}

/// The role `method` is written in as a member of its class: its own where the function has the shape that role's form needs, else a method's.
pub(super) fn written_role(method: &Method) -> Role {
    let head = &method.function.head;
    // The receiver of a method that is not static is its first parameter.
    let received = u32::from(!method.is_static && head.params > 0);
    let passed = head.params - received;
    let void = head.returns == Returns::Void;
    let shaped = match method.role {
        Role::Constructor => received == 1 && void,
        Role::StaticConstructor => head.params == 0 && void,
        Role::Finalizer => received == 1 && passed == 0 && void,
        Role::Getter => passed == 0 && !void,
        Role::Setter => passed == 1 && void,
        _ => false,
    };
    match shaped {
        true => method.role,
        false => Role::Method,
    }
}

/// Where a member stands among those of its class: constructors, the finalizer, properties, then methods.
pub(super) fn rank(method: &Method) -> u8 {
    match written_role(method) {
        Role::StaticConstructor => 0,
        Role::Constructor => 1,
        Role::Finalizer => 2,
        Role::Getter | Role::Setter => 3,
        _ => 4,
    }
}

impl Writer<'_> {
    /// Writes `method` as a member of the class whose key and name are given; as a function of the program where none is.
    pub(super) fn as_member(&mut self, method: &Method, class: Option<(TypeKey, &str)>) {
        let head = self.head;
        let body = method.function.body.as_slice();
        if let Some(reason) = &head.incomplete {
            return self.unread_function(reason);
        }
        let listed = self.function_name(head.program, head.at, &head.name);
        let waits = if method.is_async && self.sharp {
            "async "
        } else {
            ""
        };
        if method.is_async && self.sharp && self.options.on(Pass::Await) {
            self.back = async_await::handed_back(body).cloned();
            // ⚠ Only a machine that gives its task a value proves a `Task<T>`: one that gives it
            // none may be of a method of no task, which the head cannot tell.
            if let async_await::AsyncResult::TaskOf([value]) = async_await::result(body) {
                let ty = match self.kind_of(value) {
                    Kind::Unknown => "Cell".to_owned(),
                    kind => self.kind_text(&kind),
                };
                self.task = Some(format!("Task<{ty}>"));
            } else if self
                .back
                .as_ref()
                .is_some_and(|back| only_hands_back(body, back))
            {
                // A start that hands back its builder's task, and nothing else, heads a method of
                // a task, whatever the machine gives it.
                self.task = Some("Task".to_owned());
            }
        }
        // The form of old knows no class, and a function no type owns is written at the top of its file.
        let (Some((owner, class)), true) = (class, self.sharp) else {
            return self.function_as(waits, &listed, body);
        };
        // The receiver of a method that is not static is its first parameter: `this`, and not listed.
        let received = u32::from(!method.is_static && head.params > 0);
        if received == 1 {
            self.this = Some(Var::Param(0));
        }
        let fixed = if method.is_static { "static " } else { "" };
        let slot = match (method.slot, method.overrides, method.is_static) {
            (Some(_), true, false) => "override ",
            (Some(_), false, false) => "virtual ",
            _ => "",
        };
        // The user's name for the function; else the name its slot goes by wherever it is called.
        let key = FunctionKey::new(head.program, head.at);
        let unnamed = self.names.function(key).is_none() && !method.is_static;
        let slotted = method.slot.filter(|_| unnamed);
        let name = match slotted {
            Some(slot) => match self.method_name(owner, slot) {
                Some(name) => name.to_owned(),
                None => format!("Method_{slot}"),
            },
            None => listed,
        };
        // An accessor is declared under the name of its property, by the rule its uses are read by.
        let property = match classes::accessor(&name) {
            Some((_, property)) => property.to_owned(),
            None => name.clone(),
        };
        // ⚠ By the one rule the class orders its members by.
        match written_role(method) {
            Role::Constructor => {
                let text = format!("{class}({})", self.params(body));
                self.bodied(text, class, body);
            }
            Role::StaticConstructor => {
                self.bodied(format!("static {class}()"), class, body);
            }
            Role::Finalizer => {
                self.bodied(format!("~{class}()"), class, body);
            }
            Role::Getter => {
                self.property(method, slot, &property, None, body);
            }
            Role::Setter => {
                let value = Some(Var::Param(received));
                self.property(method, slot, &property, value, body);
            }
            _ => self.function_as(&format!("{fixed}{slot}{waits}"), &name, body),
        }
    }

    /// Writes a head that is the whole of its line and the body under it.
    fn bodied(&mut self, head: String, name: &str, body: &[tree::Stmt]) {
        self.headed(head, name);
        self.line(0, "{");
        self.block(body, 1);
        self.line(0, "}");
    }

    /// Writes a getter or a setter as the one accessor of a property; `value` is the setter's parameter.
    // ⚠ A getter and the setter of the same field are two properties here: nothing pairs them yet.
    fn property(
        &mut self,
        method: &Method,
        slot: &str,
        name: &str,
        value: Option<Var>,
        body: &[tree::Stmt],
    ) {
        let fixed = if method.is_static { "static " } else { slot };
        let ty = match value {
            Some(value) => self.declared_type(value),
            None => self.returned(),
        };
        self.setter = value;
        // A getter that only yields a value is that value: `T Name => e;`. One that reads a
        // local declares it, and so keeps its block.
        let only = match (value, body) {
            (None, [tree::Stmt::Return(Some(yielded))])
                if placement(body, false, None).hoisted.is_empty() =>
            {
                Some(yielded)
            }
            _ => None,
        };
        if let Some(yielded) = only {
            let yielded = self.value(yielded);
            return self.headed(format!("{fixed}{ty} {name} => {yielded};"), name);
        }
        self.headed(format!("{fixed}{ty} {name}"), name);
        self.line(0, "{");
        self.line(1, if value.is_some() { "set" } else { "get" });
        self.line(1, "{");
        self.block(body, 2);
        self.line(1, "}");
        self.line(0, "}");
    }
}

#[cfg(test)]
#[path = "print_member_tests.rs"]
mod tests;
