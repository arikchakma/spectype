//! A port of Redocly's `lintDocument` (`@redocly/openapi-core` 1.34): the visitor walk
//! openapi-typescript runs before bundling, and the built-in OAS3 rules a `redocly.yaml` turns on.
//!
//! The walk copies `walkDocument` and `normalizeVisitors` closely, because the order problems are
//! reported in (and so which error openapi-typescript throws) falls out of their details: deeper
//! nested visitors run first, each node is visited once per type by top-level visitors, and a
//! nested visitor only enters directly below its parent.

mod ajv;
mod rules;
mod structure;

use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;
use std::rc::Rc;

pub use spectype_document::Problem;

use crate::bundle::Node;
use crate::bundle::types::{Pt, Ty, apply, type_def};
use spectype_document::pointer::JsonPointer;
use spectype_document::value::{Map, Value};

/// Rule settings after Redocly merged `extends` and `rules`.
///
/// Rule id → `"error"`, `"warn"`, `"off"`, or an object with a `severity` and rule options. The default runs no rules, which is
/// what openapi-typescript's CLI does without a `redocly.yaml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// Rules for `OpenAPI` 3.0 documents.
    pub oas3_0: Map,
    /// Rules for `OpenAPI` 3.1 documents.
    pub oas3_1: Map,
    /// Problems `.redocly.lint-ignore.yaml` silences: the canonical file path, the rule id and the
    /// `#/...` pointer.
    pub ignore: std::collections::BTreeSet<(String, String, String)>,
}

impl Config {
    /// Whether no rule is enabled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        enabled(&self.oas3_0).next().is_none() && enabled(&self.oas3_1).next().is_none()
    }

    /// Enabled rules spectype does not implement, which it skips: built-in ones outside `minimal`
    /// and `recommended`, configurable `rule/…` assertions and plugin rules.
    #[must_use]
    pub fn unsupported_rules(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (id, _) in enabled(&self.oas3_0).chain(enabled(&self.oas3_1)) {
            let skipped = if rules::ORDER.contains(&id.as_str()) { !rules::is_supported(id) } else { id.contains('/') };
            if skipped && !out.contains(id) {
                out.push(id.clone());
            }
        }
        out
    }
}

fn enabled(rules: &Map) -> impl Iterator<Item = (&String, &Value)> {
    rules.iter().filter(|(_, v)| severity(v) != "off")
}

/// `getRuleSettings(...).severity`.
fn severity(setting: &Value) -> String {
    match setting {
        Value::String(s) => s.clone(),
        Value::Object(m) => m.get("severity").map_or_else(|| "error".into(), Value::js_string),
        _ => "off".into(),
    }
}

/// The documents the resolver loaded, and Redocly's `resolvedRefMap`.
pub(crate) trait Docs {
    fn root(&self) -> Node;
    /// The document's `source.absoluteRef`.
    fn source(&self, doc: usize) -> &str;
    /// Where `r`, written in `doc`, points: its document and pointer, and the node there, which
    /// is missing when only part of the pointer exists (Redocly still gives it a location).
    fn resolve(&self, doc: usize, r: &str) -> Option<(Option<Node>, usize, String)>;
    /// The loaded document whose `absoluteRef` is `source`.
    fn doc_index(&self, source: &str) -> Option<usize>;
}

/// A Redocly `Location`: a document and a lazily joined `#/...` pointer.
#[derive(Debug, Clone)]
pub(crate) struct Loc {
    pub doc: usize,
    path: Rc<LocPath>,
}

#[derive(Debug)]
enum LocPath {
    Base(String),
    /// A child of the parent path, by its unescaped key.
    Child(Rc<Self>, String, usize, bool),
}

impl LocPath {
    fn shape(&self) -> (usize, bool) {
        match self {
            Self::Base(b) => (b.len(), b.ends_with('/')),
            Self::Child(_, _, len, slash) => (*len, *slash),
        }
    }

    fn child(parent: &Rc<Self>, seg: String) -> Self {
        let (len, slash) = parent.shape();
        let sep = if len == 0 { 2 } else { usize::from(!slash) };
        let escaped = seg.len() + seg.bytes().filter(|b| matches!(b, b'~' | b'/')).count();
        Self::Child(Rc::clone(parent), seg, len + sep + escaped, escaped == 0)
    }
}

impl Loc {
    fn new(doc: usize, pointer: &str) -> Self {
        Self { doc, path: Rc::new(LocPath::Base(pointer.to_owned())) }
    }

    /// `location.child([key])`.
    pub fn child(&self, key: &str) -> Self {
        Self { doc: self.doc, path: Rc::new(LocPath::child(&self.path, key.to_owned())) }
    }

    pub fn index(&self, i: usize) -> Self {
        Self { doc: self.doc, path: Rc::new(LocPath::child(&self.path, i.to_string())) }
    }

    pub fn key_child(&self, key: &Key) -> Self {
        match key {
            Key::Idx(i) => self.index(*i),
            Key::Str(s) => self.child(s),
        }
    }

    pub fn pointer(&self) -> JsonPointer {
        let mut segs = Vec::new();
        let mut path = &self.path;
        let base = loop {
            match &**path {
                LocPath::Base(b) => break b,
                LocPath::Child(parent, seg, ..) => {
                    segs.push(seg);
                    path = parent;
                }
            }
        };
        let mut pointer = JsonPointer::from(base.as_str());
        for seg in segs.iter().rev() {
            pointer.push(seg);
        }
        pointer
    }

    fn absolute(&self, docs: &dyn Docs) -> String {
        let p = self.pointer();
        format!("{}{}", docs.source(self.doc), if p == "#/" { "" } else { &p })
    }

    fn absolute_len(&self, docs: &dyn Docs) -> usize {
        let (len, _) = self.path.shape();
        let len = if len == 2 && self.pointer() == "#/" { 0 } else { len };
        docs.source(self.doc).len() + len
    }
}

/// The key a node sits under in its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Str(String),
    Idx(usize),
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Str(s) => f.write_str(s),
            Self::Idx(i) => write!(f, "{i}"),
        }
    }
}

/// Object identity for objects and arrays, `SameValueZero` for primitives, as a JS `Set` has it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum NodeKey {
    Ptr(usize),
    Prim(u8, String),
}

pub(crate) fn node_key(n: &Node) -> NodeKey {
    match n {
        Node::Prim(v) => NodeKey::Prim(prim_tag(v), v.js_string()),
        _ => NodeKey::Ptr(n.id().unwrap_or_default()),
    }
}

fn prim_tag(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) | Value::Object(_) => 4,
    }
}

/// What a visitor is registered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    Any,
    Ref,
    T(Ty),
}

/// One level of a rule's visitor object. `enter`/`leave` are the rule's own callback ids; an
/// object visitor without `enter` still gets a no-op one, as in `normalizeVisitorLevel`. `weak`
/// lists the types `addWeakNodes` registers between the parent level and this one.
pub(crate) struct V {
    pub target: Target,
    pub enter: Option<u8>,
    pub leave: Option<u8>,
    pub weak: &'static [Ty],
    pub children: &'static [Self],
}

impl V {
    pub const fn on(target: Target, id: u8) -> Self {
        Self { target, enter: Some(id), leave: None, weak: &[], children: &[] }
    }
}

pub(crate) trait Rule {
    fn visitors(&self) -> &'static [V];
    fn visit(&mut self, id: u8, node: &Node, cx: &mut Cx<'_>);
    /// `ref` visitors get the resolved node and location too.
    fn visit_ref(&mut self, _id: u8, _node: &Node, _cx: &mut Cx<'_>, _resolved: Option<&(Node, Loc)>) {}
}

/// The visitor context: `report`, `location`, `rawLocation`, `type`, `key`, `resolve`, ...
pub(crate) struct Cx<'w> {
    pub ty: Ty,
    pub key: &'w Key,
    pub location: Loc,
    pub raw_location: Loc,
    pub parent_locations: Vec<(Ty, Loc)>,
    pub v31: bool,
    docs: &'w dyn Docs,
    sink: &'w mut Vec<Problem>,
    ignore: &'w std::collections::BTreeSet<(String, String, String)>,
    rule: &'w Active,
    ignored: Option<&'w mut bool>,
    mutated: &'w std::cell::Cell<bool>,
}

impl<'w> Cx<'w> {
    pub fn report(&mut self, message: &str) {
        let at = self.location.clone();
        self.report_at(message, &at);
    }

    pub fn report_at(&mut self, message: &str, at: &Loc) {
        let message = match &self.rule.message {
            Some(custom) => custom.replacen("{{message}}", message, 1),
            None => message.to_owned(),
        };
        if !self.ignore.is_empty() {
            let source = self.docs.source(at.doc);
            let file =
                std::fs::canonicalize(source).map_or_else(|_| source.to_owned(), |p| p.to_string_lossy().into_owned());
            if self.ignore.contains(&(file, self.rule.id.clone(), at.pointer().to_string())) {
                return;
            }
        }
        self.sink.push(Problem {
            rule: self.rule.id.clone(),
            error: self.rule.error,
            message,
            pointer: Some(at.pointer()),
        });
    }

    /// `ctx.resolve(node)` against the current document, when it finds a node.
    pub fn resolve(&self, node: &Node) -> Option<(Node, Loc)> {
        match resolve_from(self.docs, node, &self.location) {
            Some((Some(n), loc)) => Some((n, loc)),
            _ => None,
        }
    }

    /// `ctx.resolve(node).location`, which exists for a partly resolved pointer too.
    pub fn resolve_location(&self, node: &Node) -> Option<Loc> {
        resolve_from(self.docs, node, &self.location).map(|(_, loc)| loc)
    }

    pub fn absolute(&self, loc: &Loc) -> String {
        loc.absolute(self.docs)
    }

    pub fn parent_location(&self, ty: Ty) -> Option<&Loc> {
        self.parent_locations.iter().find(|(t, _)| *t == ty).map(|(_, l)| l)
    }

    /// Records that a rule changed the document, so the bundler copies it back.
    pub fn mark_mutated(&self) {
        self.mutated.set(true);
    }

    pub fn ignore_next_visitors(&mut self) {
        if let Some(flag) = self.ignored.as_deref_mut() {
            *flag = true;
        }
    }

    pub fn docs(&self) -> &'w dyn Docs {
        self.docs
    }
}

fn resolve_from(docs: &dyn Docs, node: &Node, from: &Loc) -> Option<(Option<Node>, Loc)> {
    match node.ref_str() {
        None => Some((Some(node.clone()), from.clone())),
        Some(r) => docs.resolve(from.doc, &r).map(|(n, doc, p)| (n, Loc::new(doc, &p))),
    }
}

struct Active {
    id: String,
    error: bool,
    message: Option<String>,
    rule: Box<dyn Rule>,
}

struct Context {
    rule: usize,
    target: Target,
    parent: Option<usize>,
    weak: bool,
    stack: Vec<Activation>,
    seen: FxHashSet<NodeKey>,
}

struct Activation {
    node: NodeKey,
    loc: Loc,
    next: Vec<Ty>,
    with_parent: Option<NodeKey>,
}

#[derive(Clone, Copy)]
struct Entry {
    ctx: usize,
    depth: usize,
    visit: Option<u8>,
}

const MAX_DEPTH: usize = 2_000;

/// Runs the enabled rules over the resolved document and returns their problems in report
/// order, and whether a rule changed the document (Ajv adds `"null"` to nullable type arrays).
pub(crate) fn run(docs: &dyn Docs, v31: bool, config: &Config) -> (Vec<Problem>, bool) {
    let settings = if v31 { &config.oas3_1 } else { &config.oas3_0 };
    let mut active = Vec::new();
    for id in rules::ORDER {
        let Some(setting) = settings.get(id) else { continue };
        let sev = severity(setting);
        if sev == "off" {
            continue;
        }
        let empty = Map::new();
        let opts = setting.as_object().unwrap_or(&empty);
        let Some(rule) = rules::make(id, opts) else { continue };
        let message = opts.get("message").map(Value::js_string);
        active.push(Active { id: (*id).to_owned(), error: sev == "error", message, rule });
    }
    if active.is_empty() {
        return (Vec::new(), false);
    }
    let mut w = Walker {
        docs,
        ignore: &config.ignore,
        v31,
        rules: active,
        ctxs: Vec::new(),
        enter: FxHashMap::default(),
        leave: FxHashMap::default(),
        enter_ty: FxHashMap::default(),
        leave_ty: FxHashMap::default(),
        seen: FxHashMap::default(),
        ignored: FxHashSet::default(),
        ignored_lens: FxHashSet::default(),
        reaches: FxHashMap::default(),
        problems: Vec::new(),
        keep: Vec::new(),
        mutated: std::cell::Cell::new(false),
    };
    w.normalize();
    let root = docs.root();
    w.walk(&root, Ty::Root, &Loc::new(0, "#/"), &Key::Str(String::new()), 0);
    (w.problems, w.mutated.get())
}

struct Walker<'a> {
    docs: &'a dyn Docs,
    ignore: &'a std::collections::BTreeSet<(String, String, String)>,
    v31: bool,
    rules: Vec<Active>,
    ctxs: Vec<Context>,
    enter: FxHashMap<Target, Vec<Entry>>,
    leave: FxHashMap<Target, Vec<Entry>>,
    enter_ty: FxHashMap<Target, Rc<[Entry]>>,
    leave_ty: FxHashMap<Target, Rc<[Entry]>>,
    seen: FxHashMap<Ty, FxHashSet<NodeKey>>,
    ignored: FxHashSet<String>,
    ignored_lens: FxHashSet<usize>,
    reaches: FxHashMap<Ty, bool>,
    problems: Vec<Problem>,
    keep: Vec<Node>,
    mutated: std::cell::Cell<bool>,
}

impl Walker<'_> {
    /// `normalizeVisitors`.
    fn normalize(&mut self) {
        for rule in 0..self.rules.len() {
            let visitors = self.rules[rule].rule.visitors();
            for v in visitors {
                self.level(rule, v, None, 0);
            }
        }
        for list in self.enter.values_mut() {
            list.sort_by_key(|e| std::cmp::Reverse(e.depth));
        }
        for list in self.leave.values_mut() {
            list.sort_by_key(|e| e.depth);
        }
    }

    fn level(&mut self, rule: usize, v: &V, parent: Option<usize>, depth: usize) {
        let ctx = self.ctxs.len();
        self.ctxs.push(Context {
            rule,
            target: v.target,
            parent,
            weak: false,
            stack: Vec::new(),
            seen: FxHashSet::default(),
        });
        for child in v.children {
            self.level(rule, child, Some(ctx), depth + 1);
        }
        for ty in v.weak {
            let weak = self.ctxs.len();
            self.ctxs.push(Context {
                rule,
                target: Target::T(*ty),
                parent,
                weak: true,
                stack: Vec::new(),
                seen: FxHashSet::default(),
            });
            self.enter.entry(Target::T(*ty)).or_default().push(Entry { ctx: weak, depth: 0, visit: None });
        }
        self.enter.entry(v.target).or_default().push(Entry { ctx, depth, visit: v.enter });
        if let Some(id) = v.leave {
            self.leave.entry(v.target).or_default().push(Entry { ctx, depth, visit: Some(id) });
        }
    }

    fn reaches_visitor(&mut self, ty: Ty) -> bool {
        if let Some(&r) = self.reaches.get(&ty) {
            return r;
        }
        let general = [Target::Any, Target::Ref];
        let r = if general.iter().any(|t| self.enter.contains_key(t) || self.leave.contains_key(t)) {
            true
        } else {
            let mut stack = vec![ty];
            let mut seen = FxHashSet::default();
            let mut found = false;
            while let Some(t) = stack.pop() {
                if !seen.insert(t) {
                    continue;
                }
                if self.enter.contains_key(&Target::T(t)) || self.leave.contains_key(&Target::T(t)) {
                    found = true;
                    break;
                }
                stack.extend(child_types(t, self.v31));
            }
            found
        };
        self.reaches.insert(ty, r);
        r
    }

    fn entries(&mut self, target: Target, leave: bool) -> Rc<[Entry]> {
        let cache = if leave { &self.leave_ty } else { &self.enter_ty };
        if let Some(list) = cache.get(&target) {
            return Rc::clone(list);
        }
        let map = if leave { &self.leave } else { &self.enter };
        let own = map.get(&target).map_or(&[][..], Vec::as_slice);
        let any = match target {
            Target::T(_) => map.get(&Target::Any).map_or(&[][..], Vec::as_slice),
            _ => &[],
        };
        let list: Rc<[Entry]> = if leave { [own, any].concat() } else { [any, own].concat() }.into();
        let cache = if leave { &mut self.leave_ty } else { &mut self.enter_ty };
        cache.insert(target, Rc::clone(&list));
        list
    }

    fn parent_active(&self, ctx: usize) -> Option<&Activation> {
        self.ctxs[ctx].parent.and_then(|p| self.ctxs[p].stack.last())
    }

    fn parent_locations(&self, ctx: usize) -> Vec<(Ty, Loc)> {
        let mut out = Vec::new();
        let mut cur = ctx;
        while let Some(p) = self.ctxs[cur].parent {
            if let (Target::T(t), Some(a)) = (self.ctxs[p].target, self.ctxs[p].stack.last()) {
                out.push((t, a.loc.clone()));
            }
            cur = p;
        }
        out
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn call(&mut self, e: Entry, node: &Node, ty: Ty, loc: &Loc, raw_loc: &Loc, key: &Key, ignored: &mut bool) {
        let Some(id) = e.visit else { return };
        let parent_locations = self.parent_locations(e.ctx);
        let rule = self.ctxs[e.ctx].rule;
        let Walker { rules, problems, docs, v31, mutated, ignore, .. } = self;
        let Active { rule: imp, .. } = &mut rules[rule];
        let mut imp = std::mem::replace(imp, Box::new(Nop));
        let mut cx = Cx {
            ty,
            key,
            location: loc.clone(),
            raw_location: raw_loc.clone(),
            parent_locations,
            v31: *v31,
            docs: *docs,
            sink: problems,
            ignore,
            rule: &rules[rule],
            ignored: Some(ignored),
            mutated,
        };
        imp.visit(id, node, &mut cx);
        rules[rule].rule = imp;
    }

    fn call_ref(&mut self, e: Entry, node: &Node, ty: Ty, loc: &Loc, key: &Key, resolved: Option<&(Node, Loc)>) {
        let Some(id) = e.visit else { return };
        let rule = self.ctxs[e.ctx].rule;
        let Walker { rules, problems, docs, v31, mutated, ignore, .. } = self;
        let mut imp = std::mem::replace(&mut rules[rule].rule, Box::new(Nop));
        let mut cx = Cx {
            ty,
            key,
            location: loc.clone(),
            raw_location: loc.clone(),
            parent_locations: Vec::new(),
            v31: *v31,
            docs: *docs,
            sink: problems,
            ignore,
            rule: &rules[rule],
            ignored: None,
            mutated,
        };
        imp.visit_ref(id, node, &mut cx, resolved);
        rules[rule].rule = imp;
    }

    /// `walkNode`.
    fn walk(&mut self, node: &Node, ty: Ty, location: &Loc, key: &Key, depth: usize) {
        if depth > MAX_DEPTH || !self.reaches_visitor(ty) {
            return;
        }
        let resolved = match resolve_from(self.docs, node, location) {
            Some((Some(n), loc)) => Some((n, loc)),
            _ => None,
        };
        let is_ref = node.is_ref();
        let mut entered: FxHashSet<usize> = FxHashSet::default();
        if is_ref {
            let list = self.entries(Target::Ref, false);
            for &e in &*list {
                entered.insert(e.ctx);
                self.call_ref(e, node, ty, location, key, resolved.as_ref());
            }
        }
        if let Some((rn, rl)) = resolved.as_ref().filter(|_| ty != Ty::Scalar) {
            let rkey = node_key(rn);
            let raw_key = node_key(node);
            let is_seen = self.seen.get(&ty).is_some_and(|s| s.contains(&rkey));
            let mut visited_by_some = false;
            let mut activated: Vec<usize> = Vec::new();
            let list = self.entries(Target::T(ty), false);
            let mut abs: Option<String> = None;
            let mut abs_len: Option<usize> = None;
            for &e in &*list {
                if !self.ignored.is_empty()
                    && self.ignored_lens.contains(abs_len.get_or_insert_with(|| rl.absolute_len(self.docs)))
                {
                    let a = abs.get_or_insert_with(|| rl.absolute(self.docs));
                    if self.ignored.contains(a.as_str()) {
                        break;
                    }
                }
                let ctx = &self.ctxs[e.ctx];
                if ctx.weak {
                    let open = self.parent_active(e.ctx).is_some_and(|a| a.next.is_empty());
                    if open && !self.ctxs[e.ctx].seen.contains(&raw_key) {
                        self.ctxs[e.ctx].seen.insert(raw_key.clone());
                        visited_by_some = true;
                        activated.push(e.ctx);
                    }
                    continue;
                }
                let enter = match ctx.parent {
                    Some(_) => self.parent_active(e.ctx).is_some_and(|pa| {
                        ctx.stack.last().and_then(|a| a.with_parent.as_ref()) != Some(&pa.node)
                            && pa.next.last() != Some(&ty)
                    }),
                    None => !is_seen,
                };
                if !enter {
                    continue;
                }
                activated.push(e.ctx);
                let with_parent = self.parent_active(e.ctx).map(|a| a.node.clone());
                self.ctxs[e.ctx].stack.push(Activation {
                    node: rkey.clone(),
                    loc: rl.clone(),
                    next: Vec::new(),
                    with_parent,
                });
                let mut up = self.ctxs[e.ctx].parent;
                while let Some(p) = up {
                    if let Some(a) = self.ctxs[p].stack.last_mut() {
                        a.next.push(ty);
                    }
                    up = self.ctxs[p].parent;
                }
                visited_by_some = true;
                entered.insert(e.ctx);
                let mut ignore = false;
                self.call(e, rn, ty, rl, location, key, &mut ignore);
                if ignore {
                    self.ignored_lens.insert(*abs_len.get_or_insert_with(|| rl.absolute_len(self.docs)));
                    self.ignored.insert(abs.get_or_insert_with(|| rl.absolute(self.docs)).clone());
                }
            }
            if visited_by_some || !is_seen {
                self.seen.entry(ty).or_default().insert(rkey.clone());
                self.descend(node, rn, ty, location, rl, depth);
            }
            for &c in activated.iter().rev() {
                if self.ctxs[c].weak {
                    self.ctxs[c].seen.remove(&rkey);
                    continue;
                }
                self.ctxs[c].stack.pop();
                let mut up = self.ctxs[c].parent;
                while let Some(p) = up {
                    if let Some(a) = self.ctxs[p].stack.last_mut() {
                        a.next.pop();
                    }
                    up = self.ctxs[p].parent;
                }
            }
            let list = self.entries(Target::T(ty), true);
            for &e in &*list {
                if !self.ctxs[e.ctx].weak && entered.contains(&e.ctx) {
                    let mut ignore = false;
                    self.call(e, rn, ty, rl, location, key, &mut ignore);
                }
            }
        }
        if is_ref {
            let list = self.entries(Target::Ref, true);
            for &e in &*list {
                if entered.contains(&e.ctx) {
                    self.call_ref(e, node, ty, location, key, resolved.as_ref());
                }
            }
        }
    }

    /// The child walk of `walkNode`: type-tree properties first, then the rest.
    fn descend(&mut self, node: &Node, rn: &Node, ty: Ty, loc: &Loc, rl: &Loc, depth: usize) {
        let td = type_def(ty, self.v31);
        match rn {
            Node::Arr(a) => {
                let Some(item_ty) = td.items else { return };
                let len = a.borrow().len();
                for i in 0..len {
                    let item = a.borrow().get(i).cloned();
                    if let Some(item) = item {
                        self.walk(&item, item_ty, &rl.index(i), &Key::Idx(i), depth + 1);
                    }
                }
            }
            Node::Obj(m) => {
                let mut extra: Vec<String> = Vec::new();
                if td.additional.is_some() {
                    extra.extend(m.borrow().keys().filter(|k| !td.props.iter().any(|(p, _)| p == k)).cloned());
                } else if td.ext {
                    extra.extend(m.borrow().keys().filter(|k| k.starts_with("x-")).cloned());
                }
                let raw = match node {
                    Node::Obj(raw) if node.is_ref() => Some(Rc::clone(raw)),
                    _ => None,
                };
                if let Some(raw) = &raw {
                    let siblings: Vec<String> = raw
                        .borrow()
                        .keys()
                        .filter(|k| *k != "$ref" && !td.props.iter().any(|(p, _)| p == k) && !extra.contains(k))
                        .cloned()
                        .collect();
                    extra.extend(siblings);
                }
                let mut present: Vec<usize> = Vec::new();
                let mut collect = |keys: Vec<String>| {
                    for k in keys {
                        if let Some(i) = td.props.iter().position(|(p, _)| *p == k)
                            && !present.contains(&i)
                        {
                            present.push(i);
                        }
                    }
                };
                collect(m.borrow().keys().cloned().collect());
                if let Some(raw) = &raw {
                    collect(raw.borrow().keys().cloned().collect());
                }
                present.sort_unstable();
                let props: Vec<(String, Option<crate::bundle::types::P>)> = present
                    .into_iter()
                    .map(|i| (td.props[i].0.to_owned(), Some(td.props[i].1)))
                    .chain(extra.into_iter().map(|k| (k, None)))
                    .collect();
                for (prop, raw_p) in props {
                    let (value, at) = match rn.get(&prop) {
                        Some(v) => (v, rl),
                        None => match node.get(&prop) {
                            Some(v) => (v, loc),
                            None => continue,
                        },
                    };
                    let pt = match raw_p {
                        Some(p) => apply(p, &value, &prop),
                        None => td.prop(&prop, &value),
                    };
                    let (prop_ty, value) = match pt {
                        Some(Pt::Ty(t)) => (t, value),
                        Some(Pt::Direct) => {
                            let fresh = Node::new_obj([("$ref".to_owned(), value)]);
                            self.keep.push(fresh.clone());
                            (Ty::Schema, fresh)
                        }
                        _ => continue,
                    };
                    if prop_ty == Ty::Scalar && !value.is_ref() {
                        continue;
                    }
                    self.walk(&value, prop_ty, &at.child(&prop), &Key::Str(prop.clone()), depth + 1);
                }
            }
            Node::Prim(_) => {}
        }
    }
}

fn child_types(ty: Ty, v31: bool) -> Vec<Ty> {
    use crate::bundle::types::P;
    let td = type_def(ty, v31);
    let mut out: Vec<Ty> = td.items.into_iter().chain([Ty::Scalar]).collect();
    if td.ext {
        out.push(Ty::SpecExtension);
    }
    for p in td.props.iter().map(|(_, p)| *p).chain(td.additional) {
        match p {
            P::T(t) | P::BoolOr(t) => out.push(t),
            P::Items30 => out.extend([Ty::SchemaList, Ty::Schema]),
            P::PathKey => out.push(Ty::PathItem),
            P::ResponseKey => out.push(Ty::Response),
            P::Mapping => out.push(Ty::Schema),
            P::S | P::Null => {}
        }
    }
    out
}

struct Nop;

impl Rule for Nop {
    fn visitors(&self) -> &'static [V] {
        &[]
    }
    fn visit(&mut self, _: u8, _: &Node, _: &mut Cx<'_>) {}
}

/// `String(value)`.
pub(crate) fn js_string(n: &Node) -> String {
    match n {
        Node::Prim(v) => v.js_string(),
        Node::Arr(a) => a
            .borrow()
            .iter()
            .map(|v| if matches!(v, Node::Prim(Value::Null)) { String::new() } else { js_string(v) })
            .collect::<Vec<_>>()
            .join(","),
        Node::Obj(_) => "[object Object]".into(),
    }
}

/// Redocly's `oasTypeOf`.
pub(crate) fn oas_type_of(n: &Node) -> &'static str {
    match n {
        Node::Arr(_) => "array",
        Node::Prim(Value::Null) => "null",
        Node::Prim(Value::Number(x)) if x.is_finite() && x.fract() == 0.0 => "integer",
        Node::Prim(Value::Number(_)) => "number",
        Node::Prim(Value::String(s)) if is_native_function(s) => "function",
        Node::Prim(Value::String(_)) => "string",
        Node::Prim(Value::Bool(_)) => "boolean",
        Node::Obj(_) | Node::Prim(_) => "object",
    }
}

/// Redocly's `matchesJsonSchemaType`.
pub(crate) fn matches_type(n: &Node, ty: &str, nullable: bool) -> bool {
    if nullable && matches!(n, Node::Prim(Value::Null)) {
        return true;
    }
    match ty {
        "array" => matches!(n, Node::Arr(_)),
        "object" => matches!(n, Node::Obj(_)),
        "null" => matches!(n, Node::Prim(Value::Null)),
        "integer" => oas_type_of(n) == "integer",
        "number" => matches!(n, Node::Prim(Value::Number(_))),
        "string" => matches!(n, Node::Prim(Value::String(s)) if !is_native_function(s)),
        "boolean" => matches!(n, Node::Prim(Value::Bool(_))),
        _ => false,
    }
}

/// The bundler resolves pointers into `Object.prototype` to the function's source text.
fn is_native_function(s: &str) -> bool {
    s.starts_with("function ") && s.ends_with("() { [native code] }")
}

pub(crate) fn is_plain_object(n: &Node) -> bool {
    matches!(n, Node::Obj(_))
}

/// `value[key]` is truthy.
pub(crate) fn truthy_at(n: &Node, key: &str) -> bool {
    n.get(key).is_some_and(|v| v.truthy())
}

pub(crate) fn str_at(n: &Node, key: &str) -> Option<String> {
    n.get(key).and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_shape_matches_its_pointer() {
        for base in ["", "#", "#/", "#/a", "#/a/", "#foo"] {
            let mut loc = Loc::new(0, base);
            for seg in ["x", "", "a/b~c", "", "~", "y"] {
                loc = loc.child(seg);
                let pointer = loc.pointer();
                assert_eq!(loc.path.shape(), (pointer.len(), pointer.ends_with('/')), "{base} {pointer}");
            }
        }
    }
}
