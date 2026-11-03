// SPDX-License-Identifier: BSD-2-Clause
//! Applying a validated patch to a scene, atomically. Every change is recorded in a journal; the
//! first op that cannot apply rolls the journal back, so the scene is exactly as before and the
//! caller gets one sentence. FREE is deferred to the end for the same reason. The journal is a
//! reused buffer: a steady-state apply allocates nothing.

use super::arena::{AnimReq, Scene};
use super::ids::{Idx, MAX_DEPTH, MAX_NODES, NIL, NodeId};
use super::kind::{Kind, dirty, state};
use super::patch::{APPEND, MAX_CLASSES, Op, Patch, PropValue};
use super::prop::{self, Ty, id as pid};
use super::read::SceneRead;
use super::strings::StrId;
use crate::error::{Error, Result};
use crate::scene::ids::{Ix, to_u8, to_u16, to_u32};

#[derive(Clone, Copy)]
enum Undo {
    Created(Idx),
    Inserted(Idx),
    Detached {
        node: Idx,
        parent: Idx,
        before: Idx,
    },
    Prop {
        node: Idx,
        prop: u16,
        old: Option<PropValue>,
    },
    Text {
        node: Idx,
        old: StrId,
    },
    Classes {
        node: Idx,
        n: u8,
        old: [u32; MAX_CLASSES],
    },
    State {
        node: Idx,
        old: u32,
    },
    Key {
        node: Idx,
        old: u64,
    },
    Events {
        node: Idx,
        old: u32,
    },
    A11y {
        node: Idx,
        role: u8,
        name: StrId,
        desc: StrId,
    },
    Focus(Idx),
    Doomed(Idx),
}

/// Reused by every apply on one window.
#[derive(Default)]
pub struct Journal {
    undo: Vec<Undo>,
    doomed: Vec<Idx>,
    /// Strings replaced by the patch; released at commit.
    old_strings: Vec<StrId>,
    anims_before: usize,
    seq_before: u32,
}

/// What an apply did, for the window's bookkeeping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub ops: u32,
    pub created: u32,
    pub freed: u32,
    pub focus_requested: bool,
}

fn gone(what: &str) -> Error {
    Error::invalid(format!(
        "{what} is not a part of this window (it was never created, or was freed)."
    ))
}

impl Scene {
    fn resolve(&self, raw: u32) -> Result<Idx> {
        let id = NodeId(raw);
        let slot = id.slot();
        if (slot.ix()) >= self.live.len()
            || !self.live[slot.ix()]
            || self.gen_[slot.ix()] != id.generation()
        {
            return Err(gone(&format!("Part {raw}")));
        }
        if self.doomed[slot.ix()] {
            return Err(Error::invalid(format!(
                "Part {raw} was freed earlier in this patch."
            )));
        }
        Ok(slot)
    }

    /// Apply `patch` and keep it: [`Scene::try_apply`] then [`Scene::commit`].
    pub fn apply(&mut self, patch: &Patch<'_>, j: &mut Journal) -> Result<Applied> {
        let a = self.try_apply(patch, j)?;
        self.commit(j);
        Ok(a)
    }

    /// Apply `patch` but leave it undoable: the caller finishes with [`Scene::commit`] or
    /// [`Scene::rollback`]. On an error here nothing has changed (already rolled back).
    pub fn try_apply(&mut self, patch: &Patch<'_>, j: &mut Journal) -> Result<Applied> {
        j.undo.clear();
        j.doomed.clear();
        j.old_strings.clear();
        j.anims_before = self.anims.len();
        j.seq_before = self.last_seq;
        let mut applied = Applied::default();
        for op in patch.ops() {
            if let Err(e) = self.apply_op(&op, j, &mut applied) {
                self.rollback(j);
                return Err(e);
            }
            applied.ops += 1;
        }
        self.last_seq = patch.seq;
        self.version += 1;
        Ok(applied)
    }

    /// Keep what [`Scene::try_apply`] did: free the doomed parts and the replaced text.
    pub fn commit(&mut self, j: &mut Journal) -> u32 {
        for s in j.old_strings.drain(..) {
            self.strings.release(s);
        }
        let mut freed = 0;
        for slot in j.doomed.drain(..) {
            let before = self.count;
            self.free_subtree(slot);
            freed += to_u32(before - self.count);
        }
        if self.focus != NIL && !self.live[self.focus.ix()] {
            self.focus = NIL;
        }
        j.undo.clear();
        freed
    }

    /// Undo everything [`Scene::try_apply`] did.
    pub fn rollback(&mut self, j: &mut Journal) {
        while let Some(u) = j.undo.pop() {
            self.undo(u);
        }
        j.old_strings.clear();
        j.doomed.clear();
        self.anims.truncate(j.anims_before);
        self.last_seq = j.seq_before;
        self.version += 1;
    }

    fn undo(&mut self, u: Undo) {
        match u {
            Undo::Created(i) => {
                self.unlink(i);
                self.release_node(i);
                self.parent[i.ix()] = NIL;
            }
            Undo::Inserted(c) => self.unlink(c),
            Undo::Detached {
                node,
                parent,
                before,
            } => self.link(parent, node, before),
            Undo::Prop { node, prop, old } => {
                // A replaced Str property: the new string reference goes, the old one returns.
                if let Some(cur) = self.prop(node, prop) {
                    self.release_if_str(prop, &cur);
                }
                match old {
                    Some(v) => {
                        self.put_prop(node, prop, &v);
                    }
                    None => self.drop_prop(node, prop),
                }
            }
            Undo::Text { node, old } => {
                let cur = self.text[node.ix()];
                self.strings.release(cur);
                self.text[node.ix()] = old;
            }
            Undo::Classes { node, n, old } => {
                self.classes[node.ix()] = old;
                self.nclass[node.ix()] = n;
            }
            Undo::State { node, old } => self.state[node.ix()] = old,
            Undo::Key { node, old } => self.key[node.ix()] = old,
            Undo::Events { node, old } => self.events[node.ix()] = old,
            Undo::A11y {
                node,
                role,
                name,
                desc,
            } => {
                let n = node.ix();
                self.strings.release(self.name[n]);
                self.strings.release(self.desc[n]);
                self.role[n] = role;
                self.name[n] = name;
                self.desc[n] = desc;
            }
            Undo::Focus(f) => self.focus = f,
            Undo::Doomed(i) => self.doomed[i.ix()] = false,
        }
    }

    fn release_if_str(&mut self, prop: u16, v: &PropValue) {
        if prop::def(u32::from(prop)).is_some_and(|d| d.ty == Ty::Str) {
            self.strings.release(v.w[0]);
        }
    }

    fn apply_op(&mut self, op: &Op<'_>, j: &mut Journal, ap: &mut Applied) -> Result<()> {
        match op {
            Op::Create { .. }
            | Op::Insert { .. }
            | Op::Remove { .. }
            | Op::Move { .. }
            | Op::Focus { .. }
            | Op::Free { .. } => self.op_tree(op, j, ap),
            Op::SetProp { .. } | Op::SetText { .. } | Op::SetStyle { .. } | Op::SetState { .. } => {
                self.op_content(op, j)
            }
            _ => self.op_meta(op, j),
        }
    }

    /// The ops that change the tree: create, place, move, focus and free.
    fn op_tree(&mut self, op: &Op<'_>, j: &mut Journal, ap: &mut Applied) -> Result<()> {
        match *op {
            Op::Create { node, kind } => {
                let id = NodeId(node);
                let slot = id.slot().ix();
                if slot == 0 {
                    return Err(Error::invalid(
                        "Part 0 is the window itself; give new parts another id.",
                    ));
                }
                if slot >= MAX_NODES {
                    return Err(Error::limit(format!(
                        "A window has at most {MAX_NODES} parts; use a list for long content."
                    )));
                }
                self.ensure(slot);
                if self.live[slot] {
                    return Err(Error::invalid(format!(
                        "Part {node} already exists; free it before reusing its id."
                    )));
                }
                let i = to_u32(slot);
                self.gen_[slot] = id.generation();
                self.live[slot] = true;
                self.kind[slot] = kind.id_u8();
                self.parent[slot] = NIL;
                self.state[slot] = state::EMPTY;
                self.height[slot] = 1;
                self.count += 1;
                self.mark(i, dirty::ALL);
                j.undo.push(Undo::Created(i));
                ap.created += 1;
            }
            Op::Insert {
                parent,
                child,
                index,
            } => {
                let (p, c) = (self.resolve(parent)?, self.resolve(child)?);
                self.attach(p, c, index, j)?;
            }
            Op::Remove { node } => {
                let n = self.resolve(node)?;
                if n == 0 {
                    return Err(Error::invalid("The window itself cannot be removed."));
                }
                self.detach(n, j);
            }
            Op::Move {
                node,
                parent,
                index,
            } => {
                let (n, p) = (self.resolve(node)?, self.resolve(parent)?);
                if n == 0 {
                    return Err(Error::invalid("The window itself cannot be moved."));
                }
                self.detach(n, j);
                self.attach(p, n, index, j)?;
            }
            Op::Focus { node } => {
                let n = if node == NodeId::ROOT.0 {
                    NIL
                } else {
                    self.resolve(node)?
                };
                if self.focus != n {
                    j.undo.push(Undo::Focus(self.focus));
                    self.focus = n;
                }
                ap.focus_requested = true;
            }
            Op::Free { node } => {
                let n = self.resolve(node)?;
                if n == 0 {
                    return Err(Error::invalid("The window itself cannot be freed."));
                }
                self.detach(n, j);
                self.doomed[n.ix()] = true;
                j.doomed.push(n);
                j.undo.push(Undo::Doomed(n));
            }
            _ => {}
        }
        Ok(())
    }

    /// The ops that set what a node shows: properties, text, classes and state.
    fn op_content(&mut self, op: &Op<'_>, j: &mut Journal) -> Result<()> {
        match *op {
            Op::SetProp {
                node,
                def,
                value,
                text,
            } => {
                let n = self.resolve(node)?;
                if !prop::applies(def.id, self.kind(n)) {
                    return Err(Error::invalid(format!(
                        "`{}` does not apply to a {} part.",
                        def.name,
                        self.kind(n).name()
                    )));
                }
                let mut v = value;
                if def.ty == Ty::Str {
                    // The scene keeps an interned id, not the pool reference.
                    v.w[0] = self.strings.intern(text).ok_or_else(|| {
                        Error::limit("The window holds too much text; free some parts.")
                    })?;
                }
                let old = self.put_prop(n, def.id, &v);
                if old == Some(v) {
                    self.release_if_str(def.id, &v);
                } else {
                    if let (Some(o), Ty::Str) = (old, def.ty) {
                        j.old_strings.push(o.w[0]);
                    }
                    j.undo.push(Undo::Prop {
                        node: n,
                        prop: def.id,
                        old,
                    });
                    self.mark(n, dirty::STYLE | prop::dirty_extra(def.id));
                }
            }
            Op::SetText { node, text } => {
                let n = self.resolve(node)?;
                if matches!(
                    self.kind(n),
                    Kind::Spacer | Kind::Divider | Kind::Canvas | Kind::Surface
                ) {
                    return Err(Error::invalid(format!(
                        "A {} part has no text.",
                        self.kind(n).name()
                    )));
                }
                let old = self.text[n.ix()];
                let mut cur = old;
                match self.set_str(&mut cur, text) {
                    None => {
                        return Err(Error::limit(
                            "The window holds too much text; free some parts.",
                        ));
                    }
                    Some(false) => {}
                    Some(true) => {
                        self.text[n.ix()] = cur;
                        j.old_strings.push(old);
                        j.undo.push(Undo::Text { node: n, old });
                        self.mark(n, dirty::LAYOUT | dirty::PAINT | dirty::A11Y);
                    }
                }
            }
            Op::SetStyle {
                node,
                n: count,
                classes,
            } => {
                let n = self.resolve(node)?;
                let (old, oldn) = (self.classes[n.ix()], self.nclass[n.ix()]);
                if old != classes || oldn != count {
                    j.undo.push(Undo::Classes {
                        node: n,
                        n: oldn,
                        old,
                    });
                    self.classes[n.ix()] = classes;
                    self.nclass[n.ix()] = count;
                    self.mark(n, dirty::STYLE);
                }
            }
            Op::SetState { node, state: bits } => {
                let n = self.resolve(node)?;
                let old = self.state[n.ix()];
                let new = (old & !state::APP) | (bits & state::APP);
                if new != old {
                    j.undo.push(Undo::State { node: n, old });
                    self.state[n.ix()] = new;
                    self.mark(n, dirty::STYLE | dirty::PAINT | dirty::A11Y);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The ops that attach behaviour: keys, event binds, accessibility, animations, list ranges.
    fn op_meta(&mut self, op: &Op<'_>, j: &mut Journal) -> Result<()> {
        match *op {
            Op::SetKey { node, key } => {
                let n = self.resolve(node)?;
                let old = self.key[n.ix()];
                if old != key {
                    j.undo.push(Undo::Key { node: n, old });
                    self.key[n.ix()] = key;
                }
            }
            Op::BindEvent { node, mask } => {
                let n = self.resolve(node)?;
                let old = self.events[n.ix()];
                if old != mask {
                    j.undo.push(Undo::Events { node: n, old });
                    self.events[n.ix()] = mask;
                }
            }
            Op::SetA11y {
                node,
                role,
                name,
                description,
            } => {
                let n = self.resolve(node)?;
                let (r, on, od) = (self.role[n.ix()], self.name[n.ix()], self.desc[n.ix()]);
                let (nn, nd) = match (self.strings.intern(name), self.strings.intern(description)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        return Err(Error::limit(
                            "The window holds too much text; free some parts.",
                        ));
                    }
                };
                // The undo restores (r, on, od) and releases what is there now.
                self.role[n.ix()] = to_u8(role);
                self.name[n.ix()] = nn;
                self.desc[n.ix()] = nd;
                j.undo.push(Undo::A11y {
                    node: n,
                    role: r,
                    name: on,
                    desc: od,
                });
                j.old_strings.push(on);
                j.old_strings.push(od);
                self.mark(n, dirty::A11Y);
            }
            Op::Anim {
                node,
                prop,
                target,
                mode,
                param,
            } => {
                let n = self.resolve(node)?;
                if self.anims.len() >= 4096 {
                    return Err(Error::limit(
                        "A window runs at most 4096 animations at once.",
                    ));
                }
                self.anims.push(AnimReq {
                    node: self.id(n),
                    prop: to_u16(prop),
                    target: f32::from_bits(target),
                    mode,
                    param,
                });
            }
            Op::ListRange {
                node,
                count,
                height,
                overscan,
            } => {
                let n = self.resolve(node)?;
                if self.kind(n) != Kind::Vlist {
                    return Err(Error::invalid("Only a list part has a row range."));
                }
                for (p, w) in [
                    (pid::LIST_COUNT, count),
                    (pid::LIST_HEIGHT, height.to_bits()),
                    (pid::LIST_OVERSCAN, overscan),
                ] {
                    let v = PropValue::new(&[w]);
                    let old = self.put_prop(n, p, &v);
                    if old != Some(v) {
                        j.undo.push(Undo::Prop {
                            node: n,
                            prop: p,
                            old,
                        });
                        self.mark(n, dirty::STYLE | dirty::LAYOUT | dirty::PAINT);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn detach(&mut self, n: Idx, j: &mut Journal) {
        let parent = self.parent[n.ix()];
        if parent == NIL {
            return;
        }
        let before = self.next[n.ix()];
        self.unlink(n);
        j.undo.push(Undo::Detached {
            node: n,
            parent,
            before,
        });
    }

    fn attach(&mut self, p: Idx, c: Idx, index: u32, j: &mut Journal) -> Result<()> {
        if self.kind(p).leaf() {
            return Err(Error::invalid(format!(
                "A {} part cannot have children.",
                self.kind(p).name()
            )));
        }
        if c == 0 {
            return Err(Error::invalid(
                "The window itself cannot be placed in another part.",
            ));
        }
        if self.parent[c.ix()] != NIL {
            return Err(Error::invalid(
                "That part is already in the window; move or remove it first.",
            ));
        }
        if self.is_ancestor(c, p) {
            return Err(Error::invalid("A part cannot be placed inside itself."));
        }
        let Some(depth) = self.depth_of(p) else {
            // The parent is not attached yet: depth is checked when its subtree is attached.
            return self.link_checked(p, c, index, j, None);
        };
        self.link_checked(p, c, index, j, Some(depth))
    }

    fn link_checked(
        &mut self,
        p: Idx,
        c: Idx,
        index: u32,
        j: &mut Journal,
        depth: Option<usize>,
    ) -> Result<()> {
        if let Some(d) = depth {
            let mut h = self.height[c.ix()].ix();
            if d + h > MAX_DEPTH {
                h = self.recount_height(c);
            }
            if d + h > MAX_DEPTH {
                return Err(Error::limit(format!(
                    "A window is at most {MAX_DEPTH} levels deep."
                )));
            }
        }
        let before = if index == APPEND {
            NIL
        } else {
            self.nth_child(p, index)
        };
        self.link(p, c, before);
        j.undo.push(Undo::Inserted(c));
        Ok(())
    }
}
