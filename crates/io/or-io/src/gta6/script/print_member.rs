//! A function written as the member of a class its role makes it: a constructor, the accessor of
//! a property, a method with what is proved of its slot.
//!
//! ⚠ A role is taken only where the function has the shape the role's form needs: a getter
//! takes nothing and yields a value, a setter takes one value and yields none. Any other
//! function is written as a method, whatever its role says.

use crate::gta6::script::class::{Method, Role};
use crate::gta6::script::ir::{FunctionKey, Returns, TypeKey, Var};
use crate::gta6::script::passes::async_await;
use crate::gta6::script::tree;
use crate::gta6::script::Pass;

use super::{placement, Writer};

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
        let passed = head.params - received;
        let void = head.returns == Returns::Void;
        match method.role {
            Role::Constructor if received == 1 && void => {
                let text = format!("{class}({})", self.params(body));
                self.bodied(text, class, body);
            }
            Role::StaticConstructor if head.params == 0 && void => {
                self.bodied(format!("static {class}()"), class, body);
            }
            Role::Finalizer if received == 1 && passed == 0 && void => {
                self.bodied(format!("~{class}()"), class, body);
            }
            Role::Getter if passed == 0 && !void => self.property(method, &listed, None, body),
            Role::Setter if passed == 1 && void => {
                self.property(method, &listed, Some(Var::Param(received)), body);
            }
            _ => {
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
                self.function_as(&format!("{fixed}{slot}{waits}"), &name, body);
            }
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
    fn property(&mut self, method: &Method, name: &str, value: Option<Var>, body: &[tree::Stmt]) {
        let fixed = if method.is_static { "static " } else { "" };
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
