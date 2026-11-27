//! The test of a node answered from the device's own ray query: the walk starts at a box the top
//! level's header states, is answered with the tree's root and a second box to come back to, and
//! is led down to each candidate the device hands over by answers that name one child each. The
//! walk's mask, faces, extent and commit stay its own.
//!
//! A query is a local of the function that holds it, so each time the walk comes back the
//! device's traversal is run to its end again, and the candidate shown is the least after the one
//! shown last by distance, instance and triangle: an order a device may hand them over in.

use naga::{
    BinaryOperator as Bo, Block, Expression as E, Handle, Scalar, Span, Statement, TypeInner,
    VectorSize,
};
use or_io::gta6::accel::layout::{
    BOX_CHILDREN, BOX_INSTANCES, HEADER_PARENTS, HEADER_ROOT, INSTANCE_BOTTOM, INVALID, QUERY_TOP,
    UNIT,
};
use or_io::gta6::accel::MISS;

use super::super::{vector, Cx, ResourceKind};

/// The departures a test of a node answered from the device's query makes from the arena's.
const APPROXIMATIONS: [&str; 3] = [
    // A candidate's distance and weights are the device's, not the instruction's to the bit.
    "a distance from the device",
    // Of two triangles at one distance the least instance, then the least triangle, is shown
    // first; a first-hit walk commits the nearest the device hands over.
    "candidates shown nearest first",
    // A device shows no triangle at the ray's near end, which the arena's test does.
    "a hit at zero distance",
];
/// The ray flag that makes every triangle a candidate the walk decides, whatever its flags.
const NO_OPAQUE: u32 = 0x02;
/// Every instance whatever its mask: the walk tests the mask itself.
const EVERY_INSTANCE: u32 = 0xFF;
/// A candidate's kind where it is a triangle.
const TRIANGLE: u32 = 1;
/// The members of naga's `RayIntersection` read: kind, distance, instance's number, triangle's
/// number, weights and facing.
const KIND: u32 = 0;
const T: u32 = 1;
const INSTANCE: u32 = 3;
const PRIMITIVE: u32 = 6;
const WEIGHTS: u32 = 7;
const FRONT: u32 = 8;

/// The query's state, an invocation's own: the candidate shown last and whether its triangle
/// answered, the distance committed, the bottom level the candidate is of, and the ray.
struct State {
    pending: Handle<naga::GlobalVariable>,
    given: Handle<naga::GlobalVariable>,
    /// Whether a candidate was shown since the walk started.
    any: Handle<naga::GlobalVariable>,
    committed: Handle<naga::GlobalVariable>,
    t: Handle<naga::GlobalVariable>,
    instance: Handle<naga::GlobalVariable>,
    primitive: Handle<naga::GlobalVariable>,
    front: Handle<naga::GlobalVariable>,
    weights: Handle<naga::GlobalVariable>,
    /// The byte address of the candidate's bottom level, [`INVALID`] where no range leads there.
    bottom: Handle<naga::GlobalVariable>,
    origin: Handle<naga::GlobalVariable>,
    direction: Handle<naga::GlobalVariable>,
}

/// The candidate a pass over the device's candidates keeps, in locals of the function.
struct Best {
    found: Handle<E>,
    t: Handle<E>,
    instance: Handle<E>,
    primitive: Handle<E>,
    front: Handle<E>,
    weights: Handle<E>,
}

impl Cx<'_> {
    /// The call of the test of a node, made only by a lane that runs: the query's state is the
    /// lane's own, and a lane the program masks off asks the device nothing.
    pub(super) fn asked(
        &mut self,
        function: Handle<naga::Function>,
        arguments: Vec<Handle<E>>,
    ) -> Handle<E> {
        for name in APPROXIMATIONS {
            self.note(name);
        }
        let result = self.b.result(E::CallResult(function));
        let call = Statement::Call {
            function,
            arguments,
            result: Some(result),
        };
        if !self.masked {
            self.b.push(call);
            return result;
        }
        let four = vector(&mut self.module, VectorSize::Quad, Scalar::U32);
        let answer = self.query_local("answer", four);
        let nothing = self.b.emit(E::ZeroValue(four));
        self.b.push(Statement::Store {
            pointer: answer,
            value: nothing,
        });
        let active = self.active();
        self.b.open();
        self.b.push(call);
        self.b.push(Statement::Store {
            pointer: answer,
            value: result,
        });
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: active,
            accept,
            reject: Block::new(),
        });
        self.b.load(answer)
    }

    fn query_local(&mut self, name: &str, ty: Handle<naga::Type>) -> Handle<E> {
        let variable = naga::LocalVariable {
            name: Some(name.to_owned()),
            ty,
            init: None,
        };
        let variable = (self.b.fun.local_variables).append(variable, Span::UNDEFINED);
        self.b.emit(E::LocalVariable(variable))
    }

    /// A private of the module, nought until written.
    fn query_private(
        &mut self,
        name: &str,
        ty: Handle<naga::Type>,
    ) -> Handle<naga::GlobalVariable> {
        let init = (self.module.global_expressions).append(E::ZeroValue(ty), Span::UNDEFINED);
        let global = naga::GlobalVariable {
            name: Some(name.to_owned()),
            space: naga::AddressSpace::Private,
            binding: None,
            ty,
            init: Some(init),
            memory_decorations: naga::MemoryDecorations::empty(),
        };
        self.module.global_variables.append(global, Span::UNDEFINED)
    }

    fn query_load(&mut self, global: Handle<naga::GlobalVariable>) -> Handle<E> {
        let pointer = self.global(global);
        self.b.load(pointer)
    }

    fn query_store(&mut self, global: Handle<naga::GlobalVariable>, value: Handle<E>) {
        let pointer = self.global(global);
        self.b.push(Statement::Store { pointer, value });
    }

    fn query_set(&mut self, pointer: Handle<E>, value: Handle<E>) {
        self.b.push(Statement::Store { pointer, value });
    }

    /// The device's structure, bound on its first use.
    fn query_structures(&mut self) -> Handle<naga::GlobalVariable> {
        if let Some(global) = self.trace.structures {
            return global;
        }
        let inner = TypeInner::AccelerationStructure {
            vertex_return: false,
        };
        let space = naga::AddressSpace::Handle;
        let kind = ResourceKind::DeviceStructure;
        let global = self.bound("structures".to_owned(), inner, space, kind, None);
        self.trace.structures = Some(global);
        global
    }

    /// The byte address of the node `pointer` names in the structure at `structure`.
    fn query_node_at(&mut self, structure: Handle<E>, pointer: Handle<E>) -> Handle<E> {
        let (three, six) = (self.b.u32(3), self.b.u32(UNIT.ilog2()));
        let unit = self.b.bin(Bo::ShiftRight, pointer, three);
        let within = self.b.bin(Bo::ShiftLeft, unit, six);
        self.b.bin(Bo::Add, structure, within)
    }

    /// The dword `field` bytes into the node at `at`.
    fn query_word(&mut self, at: Handle<E>, field: u32) -> Handle<E> {
        let by = self.b.u32(field);
        let bytes = self.b.bin(Bo::Add, at, by);
        self.arena_word(bytes)
    }

    /// Whether `pointer` names a box of either width.
    fn query_boxed(&mut self, pointer: Handle<E>) -> Handle<E> {
        let (six, four) = (self.b.u32(6), self.b.u32(4));
        let bits = self.b.bin(Bo::And, pointer, six);
        self.b.bin(Bo::Equal, bits, four)
    }

    /// Whether the node at the byte `at` is of the structure at `structure`: its nodes all lie
    /// before its parent table.
    fn query_within(&mut self, structure: Handle<E>, at: Handle<E>) -> Handle<E> {
        let table = self.query_word(structure, HEADER_PARENTS);
        let after = self.b.bin(Bo::LessEqual, structure, at);
        let into = self.b.bin(Bo::Subtract, at, structure);
        let before = self.b.bin(Bo::Less, into, table);
        self.b.both(after, before)
    }

    /// Whether the node at `at` is the one `pointer` names in the structure at `structure`.
    fn query_named(
        &mut self,
        (at, kind): (Handle<E>, Handle<E>),
        structure: Handle<E>,
        pointer: Handle<E>,
    ) -> Handle<E> {
        let there = self.query_node_at(structure, pointer);
        let here = self.b.bin(Bo::Equal, at, there);
        let seven = self.b.u32(7);
        let ty = self.b.bin(Bo::And, pointer, seven);
        let same = self.b.bin(Bo::Equal, kind, ty);
        self.b.both(here, same)
    }

    /// The child of the top-level box at `at` whose range of instances holds `instance`, the
    /// first in the node; [`INVALID`] where none does.
    fn query_child(&mut self, at: Handle<E>, instance: Handle<E>) -> Handle<E> {
        let mut answer = self.b.u32(INVALID);
        for i in (0..4).rev() {
            let child = self.query_word(at, BOX_CHILDREN + 4 * i);
            let range = self.query_word(at, BOX_INSTANCES + 4 * i);
            let f = &mut self.b;
            let (low, sixteen) = (f.u32(0xFFFF), f.u32(16));
            let first = f.bin(Bo::And, range, low);
            let last = f.bin(Bo::ShiftRight, range, sixteen);
            let from = f.bin(Bo::LessEqual, first, instance);
            let to = f.bin(Bo::LessEqual, instance, last);
            let none = f.u32(INVALID);
            let present = f.bin(Bo::NotEqual, child, none);
            let holds = f.both(from, to);
            let holds = f.both(holds, present);
            answer = f.select(holds, child, answer);
        }
        answer
    }

    /// Whether `(t, instance, primitive)` of `a` comes after that of `b`.
    fn query_later(&mut self, a: [Handle<E>; 3], b: [Handle<E>; 3]) -> Handle<E> {
        let f = &mut self.b;
        let primitive = f.bin(Bo::Greater, a[2], b[2]);
        let same = f.bin(Bo::Equal, a[1], b[1]);
        let then = f.both(same, primitive);
        let instance = f.bin(Bo::Greater, a[1], b[1]);
        let instance = f.bin(Bo::LogicalOr, instance, then);
        let same = f.bin(Bo::Equal, a[0], b[0]);
        let then = f.both(same, instance);
        let t = f.bin(Bo::Greater, a[0], b[0]);
        f.bin(Bo::LogicalOr, t, then)
    }

    fn query_answer(&mut self, four: Handle<naga::Type>, parts: [Handle<E>; 4]) -> Handle<E> {
        self.b.emit(E::Compose {
            ty: four,
            components: parts.to_vec(),
        })
    }

    fn query_give(&mut self, value: Handle<E>) {
        self.b.push(Statement::Return { value: Some(value) });
    }

    /// The statements of the test of a node answered from the device's query, written where the
    /// builder stands: what `or_io::gta6::accel::Query::answer` answers, with the top level's
    /// address in the arena's dword at [`QUERY_TOP`].
    #[allow(clippy::too_many_lines)]
    pub(super) fn query_body(&mut self, four: Handle<naga::Type>) {
        let flag = self.ty(TypeInner::Scalar(Scalar::BOOL));
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let float = self.ty(TypeInner::Scalar(Scalar::F32));
        let two = vector(&mut self.module, VectorSize::Bi, Scalar::F32);
        let three = vector(&mut self.module, VectorSize::Tri, Scalar::F32);
        self.module.generate_ray_desc_type();
        self.module.generate_ray_intersection_type();
        let s = State {
            pending: self.query_private("query_pending", flag),
            given: self.query_private("query_given", flag),
            any: self.query_private("query_any", flag),
            committed: self.query_private("query_committed", float),
            t: self.query_private("query_t", float),
            instance: self.query_private("query_instance", word),
            primitive: self.query_private("query_primitive", word),
            front: self.query_private("query_front", flag),
            weights: self.query_private("query_weights", two),
            bottom: self.query_private("query_bottom", word),
            origin: self.query_private("query_origin", three),
            direction: self.query_private("query_direction", three),
        };
        let [base, node, far, origin, direction] =
            [0, 1, 2, 3, 4].map(|k| self.b.emit(E::FunctionArgument(k)));
        let eight = self.b.u32(8);
        let structure = self.b.bin(Bo::ShiftLeft, base, eight);
        let at = self.query_node_at(structure, node);
        let seven = self.b.u32(7);
        let kind = self.b.bin(Bo::And, node, seven);
        let none = self.b.u32(INVALID);
        let invalid = self.query_answer(four, [none; 4]);

        // The top level and its two boxes; a top level that states none shows nothing.
        let first_word = self.b.u32(QUERY_TOP);
        let top = self.arena_word(first_word);
        let first = self.query_word(top, HEADER_ROOT);
        let first_at = self.query_node_at(top, first);
        let root = self.query_word(first_at, BOX_CHILDREN);
        let again = self.query_word(first_at, BOX_CHILDREN + 4);
        let again_at = self.query_node_at(top, again);
        let again_again = self.query_word(again_at, BOX_CHILDREN + 4);
        let (first_box, again_box) = (self.query_boxed(first), self.query_boxed(again));
        let looped = self.b.bin(Bo::Equal, again_again, again);
        let stated = self.b.both(first_box, again_box);
        let stated = self.b.both(stated, looped);
        let unstated = self.b.lnot(stated);
        self.b.open();
        self.query_give(invalid);
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: unstated,
            accept,
            reject: Block::new(),
        });

        let in_top = self.query_within(top, at);
        self.b.open();
        {
            let is_first = self.query_named((at, kind), top, first);
            self.b.open();
            // The walk starts: the query's far end is its far operand.
            self.query_store(s.committed, far);
            self.query_store(s.origin, origin);
            self.query_store(s.direction, direction);
            let nothing = self.b.emit(E::Literal(naga::Literal::Bool(false)));
            self.query_store(s.any, nothing);
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: is_first,
                accept,
                reject: Block::new(),
            });

            let is_again = self.query_named((at, kind), top, again);
            let (pending, given) = (self.query_load(s.pending), self.query_load(s.given));
            let shown = self.query_load(s.t);
            let (far_bits, shown_bits) = (self.b.bits(far), self.b.bits(shown));
            let same = self.b.bin(Bo::Equal, far_bits, shown_bits);
            let committed = self.b.both(pending, given);
            let committed = self.b.both(committed, same);
            let committed = self.b.both(committed, is_again);
            self.b.open();
            // The walk shortened its ray to the candidate shown: it committed it.
            self.query_store(s.committed, far);
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: committed,
                accept,
                reject: Block::new(),
            });

            let restart = self.b.bin(Bo::LogicalOr, is_first, is_again);
            self.b.open();
            self.query_next(&s, (top, root, again), (four, invalid));
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: restart,
                accept,
                reject: Block::new(),
            });

            // Any other box of the top level: the one child whose instances hold the candidate's.
            let pending = self.query_load(s.pending);
            let instance = self.query_load(s.instance);
            let child = self.query_child(at, instance);
            let boxed = self.query_boxed(kind);
            let led = self.b.both(pending, boxed);
            let child = self.b.select(led, child, none);
            let answer = self.query_answer(four, [child, none, none, none]);
            self.query_give(answer);
        }
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: in_top,
            accept,
            reject: Block::new(),
        });

        // A bottom level: a box names the candidate's triangle, and that triangle answers once.
        let (pending, bottom) = (self.query_load(s.pending), self.query_load(s.bottom));
        let inside = self.query_within(bottom, at);
        let mine = self.b.both(pending, inside);
        let primitive = self.query_load(s.primitive);
        let (one, three_c) = (self.b.u32(1), self.b.u32(3));
        let unit = self.b.bin(Bo::ShiftRight, primitive, one);
        let unit = self.b.bin(Bo::ShiftLeft, unit, three_c);
        let second = self.b.bin(Bo::And, primitive, one);
        let leaf = self.b.bin(Bo::InclusiveOr, unit, second);
        let boxed = self.query_boxed(kind);
        let leads = self.b.both(mine, boxed);
        self.b.open();
        let answer = self.query_answer(four, [leaf, none, none, none]);
        self.query_give(answer);
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: leads,
            accept,
            reject: Block::new(),
        });

        let four_types = self.b.u32(4);
        let triangle = self.b.bin(Bo::Less, kind, four_types);
        let given = self.query_load(s.given);
        let fresh = self.b.lnot(given);
        let its_own = self.query_named((at, kind), bottom, leaf);
        let ours = self.b.both(mine, triangle);
        let ours = self.b.both(ours, fresh);
        let ours = self.b.both(ours, its_own);
        self.b.open();
        {
            let yes = self.b.emit(E::Literal(naga::Literal::Bool(true)));
            self.query_store(s.given, yes);
            let (front, t) = (self.query_load(s.front), self.query_load(s.t));
            let weights = self.query_load(s.weights);
            let (plus, minus) = (self.b.f32(1.0), self.b.f32(-1.0));
            let side = self.b.select(front, plus, minus);
            let (u, v) = (self.b.index(weights, 0), self.b.index(weights, 1));
            let [t, u, v] = [t, u, v].map(|part| {
                let signed = self.b.bin(Bo::Multiply, part, side);
                self.b.bits(signed)
            });
            let side = self.b.bits(side);
            let answer = self.query_answer(four, [t, side, u, v]);
            self.query_give(answer);
        }
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: ours,
            accept,
            reject: Block::new(),
        });

        let missed = MISS.map(|part| self.b.u32(part));
        let missed = self.query_answer(four, missed);
        let answer = self.b.select(triangle, missed, invalid);
        self.query_give(answer);
    }

    /// The next candidate nearer than the distance committed, after the one shown last, as the
    /// first box's two children: the tree's root and the box to come back to. Four misses once
    /// there is none. Written where the builder stands, it returns on every way.
    #[allow(clippy::too_many_lines)]
    fn query_next(
        &mut self,
        s: &State,
        (top, root, again): (Handle<E>, Handle<E>, Handle<E>),
        (four, invalid): (Handle<naga::Type>, Handle<E>),
    ) {
        let flag = self.ty(TypeInner::Scalar(Scalar::BOOL));
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let float = self.ty(TypeInner::Scalar(Scalar::F32));
        let two = vector(&mut self.module, VectorSize::Bi, Scalar::F32);
        let query = self.ty(TypeInner::RayQuery {
            vertex_return: false,
        });
        let no = self.b.emit(E::Literal(naga::Literal::Bool(false)));
        self.query_store(s.pending, no);
        let best = Best {
            found: self.query_local("best_found", flag),
            t: self.query_local("best_t", float),
            instance: self.query_local("best_instance", word),
            primitive: self.query_local("best_primitive", word),
            front: self.query_local("best_front", flag),
            weights: self.query_local("best_weights", two),
        };
        self.query_set(best.found, no);
        let rq = self.query_local("query", query);

        let structures = self.query_structures();
        let structures = self.global(structures);
        let described = self.module.generate_ray_desc_type();
        let (flags, mask, near) = (
            self.b.u32(NO_OPAQUE),
            self.b.u32(EVERY_INSTANCE),
            self.b.f32(0.0),
        );
        let committed = self.query_load(s.committed);
        let (origin, direction) = (self.query_load(s.origin), self.query_load(s.direction));
        let descriptor = self.b.emit(E::Compose {
            ty: described,
            components: vec![flags, mask, near, committed, origin, direction],
        });
        self.b.push(Statement::RayQuery {
            query: rq,
            fun: naga::RayQueryFunction::Initialize {
                acceleration_structure: structures,
                descriptor,
            },
        });

        // Every candidate the device hands over, the walk's own choice among them kept.
        self.b.open();
        {
            let more = self.b.result(E::RayQueryProceedResult);
            self.b.push(Statement::RayQuery {
                query: rq,
                fun: naga::RayQueryFunction::Proceed { result: more },
            });
            let over = self.b.lnot(more);
            self.b.open();
            self.b.push(Statement::Break);
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: over,
                accept,
                reject: Block::new(),
            });
            let candidate = self.b.emit(E::RayQueryGetIntersection {
                query: rq,
                committed: false,
            });
            let [kind, t, instance, primitive, weights, front] =
                [KIND, T, INSTANCE, PRIMITIVE, WEIGHTS, FRONT]
                    .map(|member| self.b.index(candidate, member));
            let committed = self.query_load(s.committed);
            let f = &mut self.b;
            let (triangle, nought) = (f.u32(TRIANGLE), f.f32(0.0));
            let is_triangle = f.bin(Bo::Equal, kind, triangle);
            let after_near = f.bin(Bo::Greater, t, nought);
            let before_far = f.bin(Bo::Less, t, committed);
            let inside = f.both(is_triangle, after_near);
            let inside = f.both(inside, before_far);
            let any = self.query_load(s.any);
            let last = [s.t, s.instance, s.primitive].map(|global| self.query_load(global));
            let this = [t, instance, primitive];
            let after = self.query_later(this, last);
            let fresh = self.b.lnot(any);
            let after = self.b.bin(Bo::LogicalOr, fresh, after);
            let found = self.b.load(best.found);
            let kept = [best.t, best.instance, best.primitive].map(|local| self.b.load(local));
            let before = self.query_later(kept, this);
            let empty = self.b.lnot(found);
            let before = self.b.bin(Bo::LogicalOr, empty, before);
            let take = self.b.both(inside, after);
            let take = self.b.both(take, before);
            self.b.open();
            let yes = self.b.emit(E::Literal(naga::Literal::Bool(true)));
            self.query_set(best.found, yes);
            self.query_set(best.t, t);
            self.query_set(best.instance, instance);
            self.query_set(best.primitive, primitive);
            self.query_set(best.front, front);
            self.query_set(best.weights, weights);
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: take,
                accept,
                reject: Block::new(),
            });
        }
        let body = self.b.close();
        self.b.push(Statement::Loop {
            body,
            continuing: Block::new(),
            break_if: None,
        });

        let found = self.b.load(best.found);
        let nothing = self.b.lnot(found);
        self.b.open();
        self.query_give(invalid);
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: nothing,
            accept,
            reject: Block::new(),
        });
        let yes = self.b.emit(E::Literal(naga::Literal::Bool(true)));
        self.query_store(s.pending, yes);
        self.query_store(s.any, yes);
        self.query_store(s.given, no);
        for (global, local) in [
            (s.t, best.t),
            (s.instance, best.instance),
            (s.primitive, best.primitive),
            (s.front, best.front),
            (s.weights, best.weights),
        ] {
            let value = self.b.load(local);
            self.query_store(global, value);
        }

        // The candidate's instance, led to by the range each top-level box states, and the
        // bottom level it names.
        let walk = self.query_local("led", word);
        self.query_set(walk, root);
        self.b.open();
        {
            let node = self.b.load(walk);
            let boxed = self.query_boxed(node);
            let out = self.b.lnot(boxed);
            self.b.open();
            self.b.push(Statement::Break);
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition: out,
                accept,
                reject: Block::new(),
            });
            let at = self.query_node_at(top, node);
            let instance = self.query_load(s.instance);
            let child = self.query_child(at, instance);
            self.query_set(walk, child);
        }
        let body = self.b.close();
        self.b.push(Statement::Loop {
            body,
            continuing: Block::new(),
            break_if: None,
        });
        let node = self.b.load(walk);
        let none = self.b.u32(INVALID);
        let lost = self.b.bin(Bo::Equal, node, none);
        let at = self.query_node_at(top, node);
        let bottom = self.query_word(at, INSTANCE_BOTTOM);
        let eight = self.b.u32(8);
        let bottom = self.b.bin(Bo::ShiftLeft, bottom, eight);
        let bottom = self.b.select(lost, none, bottom);
        self.query_store(s.bottom, bottom);
        let answer = self.query_answer(four, [root, again, none, none]);
        self.query_give(answer);
    }
}
