//! A bit cast of a bit cast. A bit cast keeps every bit, a NaN's payload included, so a chain of
//! them is one cast of what the first read, and no cast at all where that already has the last
//! one's type. A conversion (`convert` stated) changes the value and is never seen through.

use std::collections::HashMap;

use naga::front::Typifier;
use naga::proc::ResolveContext;
use naga::{Expression as E, Function, Handle};

/// What the pass does to one function, worked out while the module is only read.
#[derive(Default)]
pub(super) struct Plan {
    /// Bit casts to write again: each reads what the chain of casts under it first read.
    rewrites: Vec<(Handle<E>, E)>,
    /// Bit casts whose value and type are an earlier expression's, and that expression.
    same: HashMap<Handle<E>, Handle<E>>,
}

pub(super) fn plan(module: &naga::Module, function: &Function) -> Plan {
    let mut plan = Plan::default();
    let context =
        ResolveContext::with_locals(module, &function.local_variables, &function.arguments);
    let mut types = Typifier::new();
    let Some((last, _)) = function.expressions.iter().next_back() else {
        return plan;
    };
    // A module the validator took resolves; one that does not is left as it is.
    if types.grow(last, &function.expressions, &context).is_err() {
        return plan;
    }
    // What each bit cast that stays reads through the casts under it: never a bit cast.
    let mut source: HashMap<Handle<E>, Handle<E>> = HashMap::new();
    for (at, expression) in function.expressions.iter() {
        let E::As {
            expr,
            kind,
            convert: None,
        } = *expression
        else {
            continue;
        };
        let read = plan.same.get(&expr).copied().unwrap_or(expr);
        let first = source.get(&read).copied().unwrap_or(read);
        // The same type, not only the same bits: a reader of another type would shift, compare
        // or divide by that type's rules.
        if types.get(first, &module.types) == types.get(at, &module.types) {
            plan.same.insert(at, first);
            continue;
        }
        source.insert(at, first);
        if first != expr {
            let seen_through = E::As {
                expr: first,
                kind,
                convert: None,
            };
            plan.rewrites.push((at, seen_through));
        }
    }
    plan
}

impl Plan {
    /// Writes the plan into `function`; how many casts it changed. What a cast first read is in
    /// scope wherever the cast was, so a reader turned to it reads what a statement has emitted.
    pub(super) fn apply(self, function: &mut Function) -> usize {
        let changed = self.rewrites.len() + self.same.len();
        if changed == 0 {
            return 0;
        }
        for (at, expression) in self.rewrites {
            function.expressions[at] = expression;
        }
        let mut turn = |read: &mut Handle<E>| {
            if let Some(&first) = self.same.get(read) {
                *read = first;
            }
        };
        for (_, expression) in function.expressions.iter_mut() {
            super::operands(expression, &mut turn);
        }
        super::reads(&mut function.body, &mut turn);
        changed
    }
}
