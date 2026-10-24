//! A port of Redocly's `bundle({ dereference: false })` (`@redocly/openapi-core` 1.34), the step
//! that pulls `$ref`s to other files into the root document before openapi-typescript sees it,
//! with the problems it reports. The Redocly lint that runs first lives in [`crate::lint`].
//!
//! Redocly works on shared, mutable JS objects: a hoisted component is the very object inside
//! the referenced file, and later rewrites inside that file show up in the component too. The
//! port keeps that model with reference-counted nodes and only copies back to a [`Value`] at the
//! end.

pub mod types;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

use crate::lint::Problem;
use crate::load::{Load, LoadedDocument};
use spectype_document::oas::ComponentKind;
use spectype_document::parse::{Format, parse_document};
use spectype_document::pointer::{JsonPointer, unescape_pointer};
use spectype_document::value::{JsMap, Value};
use spectype_document::{Error, Result};

use types::{Pt, Ty, component_of, type_def};

const ROOT: usize = 0;
const MAX_DEPTH: usize = 2_000;

/// A JS value with object identity.
#[derive(Debug, Clone)]
pub enum Node {
    Prim(Value),
    Arr(Rc<RefCell<Vec<Self>>>),
    Obj(Rc<RefCell<JsMap<Self>>>),
}

impl Node {
    fn from_value(v: Value) -> Self {
        match v {
            Value::Array(a) => Self::Arr(Rc::new(RefCell::new(a.into_iter().map(Self::from_value).collect()))),
            Value::Object(m) => Self::Obj(Rc::new(RefCell::new(m.map_values(Self::from_value)))),
            v => Self::Prim(v),
        }
    }

    pub(crate) fn new_obj(entries: impl IntoIterator<Item = (String, Self)>) -> Self {
        Self::Obj(Rc::new(RefCell::new(entries.into_iter().collect())))
    }

    fn into_value(self, open: &mut FxHashSet<usize>, depth: usize, max_depth: usize) -> Result<Value> {
        if depth > max_depth {
            return Err(too_deep());
        }
        let child = |n: Self, open: &mut FxHashSet<usize>| n.into_value(open, depth + 1, max_depth);
        match self {
            Self::Prim(v) => Ok(v),
            Self::Arr(a) => unshare(a, open, |items, open| {
                items.into_iter().map(|n| child(n, open)).collect::<Result<_>>().map(Value::Array)
            }),
            Self::Obj(m) => unshare(m, open, |map, open| map.try_map_values(|n| child(n, open)).map(Value::Object)),
        }
    }

    pub(crate) fn id(&self) -> Option<usize> {
        match self {
            Self::Prim(_) => None,
            Self::Arr(a) => Some(Rc::as_ptr(a).addr()),
            Self::Obj(m) => Some(Rc::as_ptr(m).addr()),
        }
    }

    fn is_bool(&self) -> bool {
        matches!(self, Self::Prim(Value::Bool(_)))
    }

    fn str(&self) -> Option<&str> {
        match self {
            Self::Prim(Value::String(s)) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<String> {
        match self {
            Self::Prim(Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub(crate) fn truthy(&self) -> bool {
        match self {
            Self::Prim(v) => v.truthy(),
            _ => true,
        }
    }

    /// An own property: `obj[key]` on objects, a canonical index on arrays.
    pub(crate) fn get(&self, key: &str) -> Option<Self> {
        self.with_child(key, Self::clone)
    }

    fn with_child<R>(&self, key: &str, f: impl FnOnce(&Self) -> R) -> Option<R> {
        match self {
            Self::Obj(m) => m.borrow().get(key).map(f),
            Self::Arr(a) => {
                let i: usize = key.parse().ok().filter(|i: &usize| i.to_string() == key)?;
                a.borrow().get(i).map(f)
            }
            Self::Prim(_) => None,
        }
    }

    fn has_name(&self) -> bool {
        matches!(self, Self::Obj(m) if m.borrow().contains_key("name"))
    }

    pub(crate) fn is_ref(&self) -> bool {
        matches!(self, Self::Obj(m) if matches!(m.borrow().get("$ref"), Some(Self::Prim(Value::String(_)))))
    }

    /// `isRef(node)`: a plain object whose `$ref` is a string.
    pub(crate) fn ref_str(&self) -> Option<String> {
        match self {
            Self::Obj(m) => m.borrow().get("$ref").and_then(Self::as_str),
            _ => None,
        }
    }
}

fn unshare<T: Clone>(
    rc: Rc<RefCell<T>>,
    open: &mut FxHashSet<usize>,
    convert: impl FnOnce(T, &mut FxHashSet<usize>) -> Result<Value>,
) -> Result<Value> {
    match Rc::try_unwrap(rc) {
        Ok(cell) => convert(cell.into_inner(), open),
        Err(shared) => {
            let id = Rc::as_ptr(&shared).addr();
            if !open.insert(id) {
                return Err(circular());
            }
            let inner = shared.borrow().clone();
            let out = convert(inner, open);
            open.remove(&id);
            out
        }
    }
}

fn circular() -> Error {
    Error::Unsupported("Bundling produced a circular document".into())
}

fn too_deep() -> Error {
    Error::Unsupported("Document is nested too deeply to bundle".into())
}

const OBJECT_PROTO: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

const ARRAY_PROTO: [&str; 39] = [
    "at",
    "concat",
    "copyWithin",
    "fill",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "lastIndexOf",
    "pop",
    "push",
    "reverse",
    "shift",
    "unshift",
    "slice",
    "sort",
    "splice",
    "includes",
    "indexOf",
    "join",
    "keys",
    "entries",
    "values",
    "forEach",
    "filter",
    "flat",
    "flatMap",
    "map",
    "every",
    "some",
    "reduce",
    "reduceRight",
    "toLocaleString",
    "toString",
    "toReversed",
    "toSorted",
    "toSpliced",
    "with",
    "constructor",
];

fn native_function(name: &str) -> Node {
    Node::Prim(Value::String(format!("function {name}() {{ [native code] }}")))
}

/// Where Redocly says a node lives: a document and a `#/...` pointer, built lazily because
/// almost no location is ever printed.
#[derive(Debug, Clone)]
struct Loc<'a> {
    doc: usize,
    path: LocPath<'a>,
}

#[derive(Debug, Clone)]
enum LocPath<'a> {
    Base(Rc<str>),
    Child(&'a Loc<'a>, Seg<'a>),
}

#[derive(Debug, Clone, Copy)]
enum Seg<'a> {
    Key(&'a str),
    Idx(usize),
}

impl<'a> Loc<'a> {
    fn new(doc: usize, pointer: Rc<str>) -> Loc<'static> {
        Loc { doc, path: LocPath::Base(pointer) }
    }

    fn child(&'a self, seg: Seg<'a>) -> Self {
        Loc { doc: self.doc, path: LocPath::Child(self, seg) }
    }

    fn last(&self) -> Option<Seg<'a>> {
        match &self.path {
            LocPath::Child(_, seg) => Some(*seg),
            LocPath::Base(_) => None,
        }
    }

    fn pointer(&self) -> JsonPointer {
        let mut segs = Vec::new();
        let mut loc = self;
        let base = loop {
            match &loc.path {
                LocPath::Base(b) => break b,
                LocPath::Child(parent, seg) => {
                    segs.push(*seg);
                    loc = parent;
                }
            }
        };
        let mut out = JsonPointer::from(&**base);
        for seg in segs.iter().rev() {
            match seg {
                Seg::Key(k) => out.push(k),
                Seg::Idx(i) => out.push(&i.to_string()),
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
enum Res {
    Found { doc: usize, pointer: Rc<str>, node: Option<Node> },
    Failed,
}

struct Doc {
    abs_ref: String,
    root: Node,
}

struct Bundler<'h> {
    load: &'h mut Load<'h>,
    docs: Vec<Doc>,
    files: FxHashMap<String, Result<usize, ()>>,
    resolved_paths: FxHashMap<String, Result<usize, ()>>,
    doc_uris: Vec<FxHashMap<String, Result<usize, ()>>>,
    canonical_dirs: FxHashMap<PathBuf, Option<String>>,
    refs: Vec<FxHashMap<String, Res>>,
    v31: bool,
    seen: FxHashSet<(Ty, usize)>,
    keep_alive: Vec<Node>,
    object_proto: Node,
    components: Option<Node>,
    dirty: bool,
    anchors_seen: bool,
    /// Where the bundler reported "Can't resolve $ref", in walk order.
    unresolved: Vec<JsonPointer>,
    warnings: Vec<Problem>,
}

/// Bundles `doc` in place and reports the first error openapi-typescript would raise from
/// Redocly's lint or bundle step.
///
/// `source` is the document's own path or URL; relative `$ref`s
/// resolve against it, and every referenced document is loaded through `load`. Without one the
/// document is treated like openapi-typescript treats stdin: as located at `<cwd>/`, so refs
/// resolve against the parent of the working directory.
pub fn bundle(
    doc: &mut Value,
    source: Option<&str>,
    load: &mut Load<'_>,
    lint: &crate::lint::Config,
) -> Result<Vec<Problem>> {
    if !doc.is_object() {
        return Ok(Vec::new());
    }
    let cwd = std::env::current_dir().map_or_else(|_| "/".into(), |p| p.to_string_lossy().into_owned());
    let abs_ref = match source {
        Some(url) if is_absolute_url(url) => url.to_owned(),
        Some(p) => node_resolve(&cwd, p),
        None => format!("{}/", cwd.trim_end_matches('/')),
    };
    let v31 = doc.get("openapi").and_then(Value::as_str).is_some_and(|v| v.starts_with("3.1") || v.starts_with("3.2"));
    let root = Node::from_value(std::mem::replace(doc, Value::Null));
    let mut b = Bundler {
        load,
        docs: vec![Doc { abs_ref, root: root.clone() }],
        files: FxHashMap::default(),
        resolved_paths: FxHashMap::default(),
        doc_uris: vec![FxHashMap::default()],
        canonical_dirs: FxHashMap::default(),
        refs: vec![FxHashMap::default()],
        v31,
        seen: FxHashSet::default(),
        keep_alive: Vec::new(),
        object_proto: Node::new_obj([]),
        components: None,
        dirty: false,
        anchors_seen: false,
        unresolved: Vec::new(),
        warnings: Vec::new(),
    };
    b.resolve_all(root.clone())?;
    let (mut problems, mutated) = if lint.is_empty() { (Vec::new(), false) } else { crate::lint::run(&b, v31, lint) };
    b.dirty |= mutated;
    if problems.iter().any(|p| p.error) {
        return Err(Error::Lint(problems));
    }
    b.seen.clear();
    if let Err(e) = b.walk(&root, Ty::Root, &Loc::new(ROOT, "#/".into()), None, 0) {
        return Err(with_problems(problems, e));
    }
    problems.append(&mut b.warnings);
    if let Some(pointer) = b.unresolved.pop() {
        let earlier = std::mem::take(&mut b.unresolved);
        if problems.is_empty() {
            return Err(Error::UnresolvedRefs { pointer, earlier });
        }
        let unresolved = |p: JsonPointer| Problem {
            rule: "bundler".into(),
            error: true,
            message: "Can't resolve $ref".into(),
            pointer: Some(p),
        };
        problems.extend(earlier.into_iter().chain([pointer]).map(unresolved));
        return Err(Error::Lint(problems));
    }
    let max_depth = if b.dirty { MAX_DEPTH } else { usize::MAX };
    drop(b);
    *doc = root.into_value(&mut FxHashSet::default(), 0, max_depth).map_err(|e| with_problems(problems.clone(), e))?;
    Ok(problems)
}

/// A bundling failure after lint warnings: openapi-typescript printed those first.
fn with_problems(mut problems: Vec<Problem>, e: Error) -> Error {
    if problems.is_empty() {
        return e;
    }
    problems.push(Problem { rule: "bundler".into(), error: true, message: e.to_string(), pointer: None });
    Error::Lint(problems)
}

impl crate::lint::Docs for Bundler<'_> {
    fn root(&self) -> Node {
        self.docs[ROOT].root.clone()
    }

    fn source(&self, doc: usize) -> &str {
        &self.docs[doc].abs_ref
    }

    fn doc_index(&self, source: &str) -> Option<usize> {
        self.docs.iter().position(|d| d.abs_ref == source)
    }

    fn resolve(&self, doc: usize, r: &str) -> Option<(Option<Node>, usize, String)> {
        match self.refs[doc].get(r) {
            Some(Res::Found { doc, pointer, node }) => Some((node.clone(), *doc, pointer.to_string())),
            _ => None,
        }
    }
}

impl Bundler<'_> {
    /// `resolveDocument`: follows every `$ref` reachable from the root, before anything is
    /// rewritten.
    fn resolve_all(&mut self, root: Node) -> Result<()> {
        let mut queue = VecDeque::from([(root, ROOT, Ty::Root)]);
        while let Some((node, doc, ty)) = queue.pop_front() {
            let mut anchors = FxHashMap::default();
            let mut pending = Vec::new();
            self.resolve_walk(&node, ty, &mut anchors, &mut pending, 0)?;
            for (ref_node, ty) in pending {
                let Some(r) = ref_node.ref_str() else { continue };
                let cached = if self.anchors_seen { None } else { self.refs[doc].get(&r).cloned() };
                let res = if let Some(res) = cached {
                    res
                } else {
                    let mut chain: Vec<usize> = ref_node.id().into_iter().collect();
                    self.follow(doc, &r, &anchors, &mut chain)?
                };
                if let Res::Found { doc: d, node: Some(n), .. } = res {
                    queue.push_back((n, d, ty));
                }
            }
        }
        Ok(())
    }

    /// Redocly dedupes this walk by type and location; before any rewrite, node identity is the
    /// same thing.
    fn resolve_walk(
        &mut self,
        node: &Node,
        ty: Ty,
        anchors: &mut FxHashMap<String, Node>,
        pending: &mut Vec<(Node, Ty)>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(too_deep());
        }
        let Some(id) = node.id() else { return Ok(()) };
        if !self.seen.insert((ty, id)) {
            return Ok(());
        }
        let td = type_def(ty, self.v31);
        match node {
            Node::Arr(a) => {
                let item_ty = match td.items {
                    Some(t) => t,
                    None if matches!(ty, Ty::Unknown | Ty::SpecExtension) => Ty::Unknown,
                    None => return Ok(()),
                };
                for item in a.borrow().iter() {
                    self.resolve_walk(item, item_ty, anchors, pending, depth + 1)?;
                }
            }
            Node::Obj(m) => {
                let map = m.borrow();
                if let Some(Node::Prim(a)) = map.get("$anchor")
                    && a.truthy()
                {
                    self.anchors_seen = true;
                    anchors.insert(format!("#{}", a.js_string()), node.clone());
                }
                for (key, value) in map.iter() {
                    let (prop_ty, fresh) = match td.prop(key, value) {
                        None => (Ty::Unknown, None),
                        Some(Pt::Null) => continue,
                        Some(Pt::Direct) => (Ty::Schema, Some(ref_to(value))),
                        Some(Pt::Ty(t)) => (t, None),
                    };
                    if let Some(fresh) = fresh {
                        self.keep_alive.push(fresh.clone());
                        self.resolve_walk(&fresh, prop_ty, anchors, pending, depth + 1)?;
                    } else if !matches!(value, Node::Prim(_)) {
                        self.resolve_walk(value, prop_ty, anchors, pending, depth + 1)?;
                    }
                }
                if node.is_ref() {
                    pending.push((node.clone(), ty));
                }
                if let Some(ev) = map.get("externalValue").and_then(Node::as_str) {
                    pending.push((ref_to(&Node::Prim(Value::String(ev))), ty));
                }
            }
            Node::Prim(_) => {}
        }
        Ok(())
    }

    /// `target[segment]` in JS, prototype chain included, for pointer resolution.
    fn js_get(&self, target: &Node, segment: &str) -> Option<Node> {
        if let Some(own) = target.get(segment) {
            return Some(own);
        }
        match target {
            Node::Obj(_) if segment == "__proto__" => Some(self.object_proto.clone()),
            Node::Obj(_) if segment == "constructor" => Some(native_function("Object")),
            Node::Obj(_) if OBJECT_PROTO.contains(&segment) => Some(native_function(segment)),
            Node::Arr(a) if segment == "length" => {
                #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
                let len = a.borrow().len() as f64;
                Some(Node::Prim(Value::Number(len)))
            }
            Node::Arr(_) if segment == "constructor" => Some(native_function("Array")),
            Node::Arr(_)
                if segment != "__proto__" && (ARRAY_PROTO.contains(&segment) || OBJECT_PROTO.contains(&segment)) =>
            {
                Some(native_function(segment))
            }
            _ => None,
        }
    }

    /// `followRef`: resolves `r` as written in `doc`, following refs met on the way.
    fn follow(
        &mut self,
        doc: usize,
        r: &str,
        anchors: &FxHashMap<String, Node>,
        chain: &mut Vec<usize>,
    ) -> Result<Res> {
        if is_anchor(r) {
            let res = Res::Found { doc, pointer: r.into(), node: anchors.get(r).cloned() };
            self.refs[doc].insert(r.to_owned(), res.clone());
            return Ok(res);
        }
        let (uri, segments) = split_ref(r);
        let mut target_doc = match uri {
            None => doc,
            Some(uri) => {
                let Ok(d) = self.load(doc, uri) else {
                    self.refs[doc].insert(r.to_owned(), Res::Failed);
                    return Ok(Res::Failed);
                };
                d
            }
        };
        let mut pointer = JsonPointer::from("#/");
        let mut failed = None;
        let mut target = Some(self.docs[target_doc].root.clone());
        for segment in segments {
            let Some(t) = target.take().filter(|t| t.id().is_some()) else { break };
            if let Some(child) = self.js_get(&t, &segment) {
                pointer.push(&segment);
                target = Some(child);
            } else if let Some(inner) = t.ref_str() {
                match self.follow_nested(target_doc, &t, &inner, anchors, chain)? {
                    Res::Found { doc: d, pointer: p, node } => {
                        target_doc = d;
                        let Some(child) = node.filter(|n| n.id().is_some()).and_then(|n| self.js_get(&n, &segment))
                        else {
                            pointer = JsonPointer::from(&*p);
                            break;
                        };
                        pointer = JsonPointer::from(&*p);
                        pointer.push(&segment);
                        target = Some(child);
                    }
                    f @ Res::Failed => {
                        failed = Some(f);
                        break;
                    }
                }
            } else {
                break;
            }
        }
        let mut res = failed.unwrap_or_else(|| Res::Found {
            doc: target_doc,
            pointer: pointer.as_str().into(),
            node: target.clone(),
        });
        if let (Res::Found { .. }, Some(t)) = (&res, &target)
            && let Some(inner) = t.ref_str()
        {
            res = self.follow_nested(target_doc, t, &inner, anchors, chain)?;
        }
        self.refs[doc].insert(r.to_owned(), res.clone());
        Ok(res)
    }

    fn follow_nested(
        &mut self,
        doc: usize,
        node: &Node,
        r: &str,
        anchors: &FxHashMap<String, Node>,
        chain: &mut Vec<usize>,
    ) -> Result<Res> {
        let id = node.id().unwrap_or_default();
        if chain.contains(&id) || chain.len() > MAX_DEPTH {
            return Err(Error::Unsupported("Self-referencing circular pointer".into()));
        }
        chain.push(id);
        let res = self.follow(doc, r, anchors, chain);
        chain.pop();
        res
    }

    /// `BaseResolver.resolveDocument`: `Err` for a document that can't be loaded or parsed.
    fn load(&mut self, base: usize, uri: String) -> Result<usize, ()> {
        if let Some(cached) = self.doc_uris[base].get(&uri) {
            return *cached;
        }
        let loaded = self.load_uncached(base, &uri);
        self.doc_uris[base].insert(uri, loaded);
        loaded
    }

    fn load_uncached(&mut self, base: usize, uri: &str) -> Result<usize, ()> {
        let base_ref = &self.docs[base].abs_ref;
        let abs = if is_absolute_url(uri) {
            uri.to_owned()
        } else if is_absolute_url(base_ref) {
            url::Url::parse(base_ref).and_then(|b| b.join(uri)).map_err(|_| ())?.into()
        } else {
            node_resolve(&dirname(base_ref), uri)
        };
        if let Some(cached) = self.resolved_paths.get(&abs) {
            return *cached;
        }
        let key = if is_absolute_url(&abs) { abs.clone() } else { self.canonical_path(&abs) };
        if let Some(cached) = self.files.get(&key).copied() {
            self.resolved_paths.insert(abs, cached);
            return cached;
        }
        let loaded = match (self.load)(&abs) {
            Ok(Some(doc)) => parse_loaded(&abs, doc),
            Ok(None) | Err(_) => Err(()),
        };
        let loaded = loaded.map(|value| {
            self.docs.push(Doc { abs_ref: abs.clone(), root: Node::from_value(value) });
            self.refs.push(FxHashMap::default());
            self.doc_uris.push(FxHashMap::default());
            self.docs.len() - 1
        });
        self.files.insert(key, loaded);
        self.resolved_paths.insert(abs, loaded);
        loaded
    }

    fn canonical_path(&mut self, abs: &str) -> String {
        let canonicalize = |p: &Path| std::fs::canonicalize(p).ok().map(|c| c.to_string_lossy().into_owned());
        let path = Path::new(abs);
        let is_file = std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_symlink());
        if let (true, Some(dir), Some(name)) = (is_file, path.parent(), path.file_name()) {
            let dir = self.canonical_dirs.entry(dir.to_path_buf()).or_insert_with(|| canonicalize(dir));
            if let Some(dir) = dir {
                return Path::new(&dir).join(name).to_string_lossy().into_owned();
            }
        }
        canonicalize(path).unwrap_or_else(|| abs.to_owned())
    }

    fn lookup(&self, doc: usize, r: &str) -> Option<(Node, Loc<'static>)> {
        match self.refs[doc].get(r) {
            Some(Res::Found { doc, pointer, node: Some(node) }) => {
                Some((node.clone(), Loc::new(*doc, Rc::clone(pointer))))
            }
            _ => None,
        }
    }

    /// `walkNode` with the bundle visitor: children in type-tree order, then `ref.leave`.
    fn walk(&mut self, node: &Node, ty: Ty, loc: &Loc<'_>, parent: Option<&Node>, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(too_deep());
        }
        let ref_lookup = match node {
            Node::Obj(m) => match m.borrow().get("$ref") {
                Some(Node::Prim(Value::String(r))) => Some(self.lookup(loc.doc, r)),
                _ => None,
            },
            _ => None,
        };
        let is_ref = ref_lookup.is_some();
        let resolved = ref_lookup.unwrap_or_else(|| Some((node.clone(), loc.clone())));
        if let Some((rn, rl)) = &resolved
            && ty != Ty::Scalar
            && let Some(id) = rn.id()
            && self.seen.insert((ty, id))
        {
            self.keep_alive.push(rn.clone());
            if ty == Ty::Root {
                self.enter_root(rn);
            }
            self.walk_children(node, rn, ty, loc, rl, is_ref, depth)?;
            match ty {
                Ty::Example => self.leave_example(rn, rl),
                Ty::DiscriminatorMapping => self.leave_mapping(rn, rl)?,
                _ => {}
            }
        } else if let Some((rn @ Node::Prim(Value::String(_)), rl)) = &resolved
            && ty == Ty::DiscriminatorMapping
        {
            self.leave_mapping(rn, rl)?;
        }
        if is_ref {
            let Some((rn, rl)) = resolved else {
                self.unresolved.push(loc.pointer());
                return Ok(());
            };
            if rl.doc == ROOT && loc.doc == ROOT && ty != Ty::Scalar {
                return Ok(());
            }
            self.dirty = true;
            match component_of(ty) {
                None => replace_ref(node, &rn, parent, loc.last()),
                Some(kind) => {
                    let new_ref = self.save_component(kind, &rn, &rl, loc)?;
                    if let Node::Obj(m) = node {
                        m.borrow_mut().insert("$ref".into(), Node::Prim(Value::String(new_ref.clone())));
                    }
                    let res = Res::Found { doc: ROOT, pointer: new_ref.as_str().into(), node: Some(rn) };
                    self.refs[loc.doc].insert(new_ref, res);
                }
            }
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn walk_children(
        &mut self,
        node: &Node,
        rn: &Node,
        ty: Ty,
        loc: &Loc<'_>,
        rl: &Loc<'_>,
        is_ref: bool,
        depth: usize,
    ) -> Result<()> {
        let td = type_def(ty, self.v31);
        match rn {
            Node::Arr(a) => {
                let Some(item_ty) = td.items else { return Ok(()) };
                let len = a.borrow().len();
                for i in 0..len {
                    let item = a.borrow().get(i).filter(|n| !matches!(n, Node::Prim(_))).cloned();
                    if let Some(item) = item {
                        self.walk(&item, item_ty, &rl.child(Seg::Idx(i)), Some(rn), depth + 1)?;
                    }
                }
            }
            Node::Obj(m) => {
                let mut extra: Vec<String> = Vec::new();
                if td.additional.is_some() {
                    extra.extend(
                        m.borrow().keys().filter(|k| !td.props.iter().any(|(p, _)| types::same_key(p, k))).cloned(),
                    );
                } else if td.ext {
                    extra.extend(m.borrow().keys().filter(|k| k.starts_with("x-")).cloned());
                }
                if is_ref && let Node::Obj(raw) = node {
                    let siblings: Vec<String> = raw
                        .borrow()
                        .keys()
                        .filter(|k| {
                            *k != "$ref" && !td.props.iter().any(|(p, _)| types::same_key(p, k)) && !extra.contains(k)
                        })
                        .cloned()
                        .collect();
                    extra.extend(siblings);
                }
                let mut present: Vec<usize> = Vec::new();
                let mut collect = |map: &JsMap<Node>| {
                    for k in map.keys() {
                        if let Some(i) = td.props.iter().position(|(p, _)| types::same_key(p, k))
                            && !present.contains(&i)
                        {
                            present.push(i);
                        }
                    }
                };
                collect(&m.borrow());
                if is_ref && let Node::Obj(raw) = node {
                    collect(&raw.borrow());
                }
                present.sort_unstable();
                for i in present {
                    let (prop, raw) = td.props[i];
                    self.walk_prop(node, rn, loc, rl, &td, prop, Some(raw), depth)?;
                }
                for prop in &extra {
                    self.walk_prop(node, rn, loc, rl, &td, prop, None, depth)?;
                }
            }
            Node::Prim(_) => {}
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn walk_prop(
        &mut self,
        node: &Node,
        rn: &Node,
        loc: &Loc<'_>,
        rl: &Loc<'_>,
        td: &types::TypeDef,
        prop: &str,
        raw: Option<types::P>,
        depth: usize,
    ) -> Result<()> {
        let visit = |n: &Node| {
            let pt = match raw {
                Some(p) => types::apply(p, n, prop),
                None => td.prop(prop, n),
            };
            match pt {
                Some(Pt::Ty(t)) if matches!(n, Node::Prim(_)) && t != Ty::DiscriminatorMapping => None,
                Some(Pt::Ty(t)) => Some((t, n.clone())),
                Some(Pt::Direct) => Some((Ty::Schema, ref_to(n))),
                _ => None,
            }
        };
        let (target, at) = match rn.with_child(prop, visit) {
            Some(t) => (t, rl),
            None => match node.with_child(prop, visit) {
                Some(t) => (t, loc),
                None => return Ok(()),
            },
        };
        let Some((prop_ty, value)) = target else { return Ok(()) };
        if prop_ty == Ty::Scalar && !value.is_ref() {
            return Ok(());
        }
        self.walk(&value, prop_ty, &at.child(Seg::Key(prop)), Some(rn), depth + 1)
    }

    fn enter_root(&mut self, root: &Node) {
        let Node::Obj(m) = root else { return };
        let existing = m.borrow().get("components").cloned().filter(Node::truthy);
        let components = existing.unwrap_or_else(|| {
            let c = Node::new_obj([]);
            m.borrow_mut().insert("components".into(), c.clone());
            c
        });
        self.components = Some(components);
    }

    fn leave_example(&mut self, example: &Node, at: &Loc<'_>) {
        let Node::Obj(m) = example else { return };
        let Some(ev) =
            m.borrow().get("externalValue").and_then(Node::as_str).filter(|_| !m.borrow().contains_key("value"))
        else {
            return;
        };
        if is_absolute_url(&ev) {
            return;
        }
        match self.lookup(at.doc, &ev) {
            Some((value, _)) => {
                self.dirty = true;
                let mut map = m.borrow_mut();
                map.insert("value".into(), value);
                *map = map.iter().filter(|(k, _)| *k != "externalValue").map(|(k, v)| (k.clone(), v.clone())).collect();
            }
            None => self.unresolved.push(at.pointer()),
        }
    }

    /// The bundle visitor's mapping rewrite, which stops at the first mapping that doesn't
    /// resolve. That ref was already reported once when its `{ $ref }` stand-in was walked.
    fn leave_mapping(&mut self, mapping: &Node, at: &Loc<'_>) -> Result<()> {
        let first_string = match mapping {
            Node::Prim(Value::String(s)) if !s.is_empty() => Some(0),
            Node::Arr(a) => a.borrow().iter().position(|v| v.as_str().is_some()),
            _ => None,
        };
        if let Some(i) = first_string {
            // `Object.keys` of a string or array mapping: its entries are never resolved.
            self.unresolved.push(at.child(Seg::Idx(i)).pointer());
            return Ok(());
        }
        let Node::Obj(m) = mapping else { return Ok(()) };
        let names: Vec<String> = m.borrow().keys().cloned().collect();
        for name in names {
            let Some(r) = m.borrow().get(&name).and_then(Node::as_str) else { continue };
            let Some((node, loc)) = self.lookup(at.doc, &r) else {
                self.unresolved.push(at.child(Seg::Key(&name)).pointer());
                return Ok(());
            };
            let new_ref = self.save_component(ComponentKind::Schemas, &node, &loc, at)?;
            if new_ref != r {
                self.dirty = true;
                m.borrow_mut().insert(name, Node::Prim(Value::String(new_ref)));
            }
        }
        Ok(())
    }

    /// `saveComponent`: stores `target` under `components.<kind>` and returns its new `$ref`.
    fn save_component(&mut self, kind: ComponentKind, target: &Node, at: &Loc<'_>, from: &Loc<'_>) -> Result<String> {
        let not_object = || Error::Unsupported(format!("Can't bundle into a non-object `components.{kind}`"));
        let Some(Node::Obj(components)) = self.components.clone() else { return Err(not_object()) };
        let existing = components.borrow().get(kind.as_str()).cloned();
        let group = match existing {
            Some(Node::Obj(g)) => g,
            Some(g) if g.truthy() => return Err(not_object()),
            _ => {
                self.dirty = true;
                let g = Rc::new(RefCell::new(JsMap::new()));
                components.borrow_mut().insert(kind.as_str().into(), Node::Obj(Rc::clone(&g)));
                g
            }
        };
        let name = self.component_name(&group, target, at, from);
        let same = group.borrow().get(&name).and_then(Node::id).is_some_and(|id| Some(id) == target.id());
        if !same {
            self.dirty = true;
            group.borrow_mut().insert(name.clone(), target.clone());
        }
        Ok(format!("#/components/{kind}/{name}"))
    }

    /// `getComponentName`: the shortest free pointer suffix, then `<file>_<suffix>`, then a
    /// `-2`, `-3`, … serial. A name is free when unused or already holding the same node; names
    /// of `Object.prototype` members are never free.
    fn component_name(&mut self, group: &RefCell<JsMap<Node>>, target: &Node, at: &Loc<'_>, from: &Loc<'_>) -> String {
        let free = |name: &str| {
            let existing = group.borrow().get(name).cloned();
            match existing {
                Some(e) => !e.truthy() || self.equal_or_equal_ref(&e, target, at),
                None => !OBJECT_PROTO.contains(&name),
            }
        };
        let pointer = at.pointer();
        let tail: String = pointer.chars().skip(2).collect();
        let mut parts: Vec<&str> = tail.split('/').filter(|p| !p.is_empty()).collect();
        let mut name = String::new();
        while let Some(part) = parts.pop() {
            name = if name.is_empty() { part.to_owned() } else { format!("{part}-{name}") };
            if free(&name) {
                return name;
            }
        }
        let base = ref_base_name(&self.docs[at.doc].abs_ref);
        name = if name.is_empty() { base } else { format!("{base}_{name}") };
        let prev = name.clone();
        let mut serial = 2;
        while !free(&name) {
            name = format!("{prev}-{serial}");
            serial += 1;
        }
        if serial > 2 && !group.borrow().get(&name).is_some_and(Node::truthy) {
            self.warnings.push(Problem {
                rule: "bundler".into(),
                error: false,
                message: format!(
                    "Two schemas are referenced with the same name but different content. Renamed {prev} to {name}."
                ),
                pointer: Some(from.pointer()),
            });
        }
        name
    }

    fn equal_or_equal_ref(&self, existing: &Node, target: &Node, at: &Loc<'_>) -> bool {
        if let Some(r) = existing.ref_str()
            && let Some(Res::Found { doc, pointer, .. }) = self.refs[ROOT].get(&r)
            && self.absolute_pointer(*doc, pointer) == self.absolute_pointer(at.doc, &at.pointer())
        {
            return true;
        }
        dequal(existing, target, &mut FxHashSet::default(), 0)
    }

    fn absolute_pointer(&self, doc: usize, pointer: &str) -> String {
        let pointer = if pointer == "#/" { "" } else { pointer };
        format!("{}{pointer}", self.docs[doc].abs_ref)
    }
}

/// `replaceRef`: merges the ref's sibling keys over the resolved object, or swaps a non-object
/// target into the parent.
fn replace_ref(node: &Node, resolved: &Node, parent: Option<&Node>, key: Option<Seg<'_>>) {
    match (resolved, node) {
        (Node::Obj(src), Node::Obj(dst)) => {
            if Rc::ptr_eq(src, dst) {
                return;
            }
            let mut merged: JsMap<Node> =
                dst.borrow().iter().filter(|(k, _)| *k != "$ref").map(|(k, v)| (k.clone(), v.clone())).collect();
            for (k, v) in src.borrow().iter() {
                if !merged.contains_key(k) {
                    merged.insert(k.clone(), v.clone());
                }
            }
            *dst.borrow_mut() = merged;
        }
        _ => match (parent, key) {
            (Some(Node::Obj(p)), Some(Seg::Key(k))) => p.borrow_mut().insert(k.to_owned(), resolved.clone()),
            (Some(Node::Obj(p)), Some(Seg::Idx(i))) => p.borrow_mut().insert(i.to_string(), resolved.clone()),
            (Some(Node::Arr(p)), Some(Seg::Idx(i))) => {
                if let Some(slot) = p.borrow_mut().get_mut(i) {
                    *slot = resolved.clone();
                }
            }
            _ => {}
        },
    }
}

/// `dequal`, treating a pair already being compared as equal so cyclic graphs terminate.
#[expect(clippy::float_cmp, reason = "mirrors JavaScript `===` on numbers")]
fn dequal(a: &Node, b: &Node, open: &mut FxHashSet<(usize, usize)>, depth: usize) -> bool {
    match (a, b) {
        (Node::Prim(Value::Number(x)), Node::Prim(Value::Number(y))) => x == y || (x.is_nan() && y.is_nan()),
        (Node::Prim(x), Node::Prim(y)) => x == y,
        (Node::Arr(x), Node::Arr(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let pair = (Rc::as_ptr(x).addr(), Rc::as_ptr(y).addr());
            if depth > MAX_DEPTH || !open.insert(pair) {
                return depth <= MAX_DEPTH;
            }
            let (x, y) = (x.borrow().clone(), y.borrow().clone());
            x.len() == y.len() && x.iter().zip(&y).all(|(a, b)| dequal(a, b, open, depth + 1))
        }
        (Node::Obj(x), Node::Obj(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let pair = (Rc::as_ptr(x).addr(), Rc::as_ptr(y).addr());
            if depth > MAX_DEPTH || !open.insert(pair) {
                return depth <= MAX_DEPTH;
            }
            let x: Vec<(String, Node)> = x.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let y = y.borrow().clone();
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| dequal(v, w, open, depth + 1)))
        }
        _ => false,
    }
}

/// A fresh `{ $ref: value }`, as Redocly builds for `directResolveAs` and `externalValue`.
fn ref_to(value: &Node) -> Node {
    Node::new_obj([("$ref".to_owned(), value.clone())])
}

/// `parseRef`: the part before `#/` (minus a trailing `#`) and the unescaped pointer segments.
fn split_ref(r: &str) -> (Option<String>, Vec<String>) {
    let mut parts = r.split("#/");
    let uri = parts.next().unwrap_or_default();
    let uri = uri.strip_suffix('#').unwrap_or(uri);
    let pointer = parts.next().unwrap_or_default();
    let segments =
        pointer.split('/').map(unescape_pointer).filter(|s| !s.is_empty()).map(std::borrow::Cow::into_owned).collect();
    ((!uri.is_empty()).then(|| uri.to_owned()), segments)
}

fn is_anchor(r: &str) -> bool {
    let mut chars = r.chars();
    chars.next() == Some('#')
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

fn is_absolute_url(r: &str) -> bool {
    r.starts_with("http://") || r.starts_with("https://")
}

/// `refBaseName`: the file name without its extension.
fn ref_base_name(r: &str) -> String {
    let last = r.rsplit(['/', '\\']).next().unwrap_or_default();
    match last.rfind('.') {
        Some(i) if i + 1 < last.len() => last[..i].to_owned(),
        _ => last.to_owned(),
    }
}

/// Node's `path.dirname` for Windows paths.
#[cfg(windows)]
fn dirname(p: &str) -> String {
    std::path::Path::new(p).parent().map_or_else(|| ".".into(), |d| d.to_string_lossy().into_owned())
}

/// Node's `path.dirname` for POSIX paths.
#[cfg(not(windows))]
fn dirname(p: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        return if p.starts_with('/') { "/".into() } else { ".".into() };
    }
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].trim_end_matches('/').to_owned(),
        None => ".".into(),
    }
}

/// Node's `path.resolve(base, p)` for Windows paths, with `base` already absolute.
#[cfg(windows)]
fn node_resolve(base: &str, p: &str) -> String {
    let joined = std::path::Path::new(base).join(p);
    std::path::absolute(&joined).unwrap_or(joined).to_string_lossy().into_owned()
}

/// Node's `path.resolve(base, p)` for POSIX paths, with `base` already absolute.
#[cfg(not(windows))]
fn node_resolve(base: &str, p: &str) -> String {
    let joined = if p.starts_with('/') { p.to_owned() } else { format!("{base}/{p}") };
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

/// `BaseResolver.parseDocument`: YAML/JSON by extension or MIME type, anything else as text.
/// Files get their CRLFs normalized, as Redocly's file loader does.
fn parse_loaded(abs: &str, doc: LoadedDocument) -> Result<Value, ()> {
    let text = if is_absolute_url(abs) { doc.text } else { doc.text.replace("\r\n", "\n") };
    let ext = abs.rfind('.').map_or("", |i| &abs[i..]);
    let mime = doc.mime_type.unwrap_or_default();
    if matches!(ext, ".json" | ".yml" | ".yaml") || ["json", "yaml", "openapi"].iter().any(|m| mime.contains(m)) {
        parse_document(&text, Format::Yaml).map_err(|_| ())
    } else {
        Ok(Value::String(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_convert_in_place() {
        assert_eq!(std::mem::size_of::<Node>(), std::mem::size_of::<Value>());
    }

    #[test]
    fn colliding_names_from_two_files() {
        let dir = std::env::temp_dir().join(format!("spectype-bundle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pet = |kind: &str| format!(r#"{{"components":{{"schemas":{{"Pet":{{"type":"{kind}"}}}}}}}}"#);
        std::fs::write(dir.join("a.json"), pet("string")).unwrap();
        std::fs::write(dir.join("b.json"), pet("number")).unwrap();
        let mut doc = parse_document(
            r#"{"openapi":"3.1.0","paths":{},"components":{"schemas":{
                "A":{"$ref":"a.json#/components/schemas/Pet"},
                "B":{"$ref":"./b.json#/components/schemas/Pet"},
                "C":{"items":{"$ref":"a.json#/components/schemas/Pet"}}}}}"#,
            Format::Json,
        )
        .unwrap();
        bundle(
            &mut doc,
            Some(&dir.join("root.json").to_string_lossy()),
            &mut crate::load_file,
            &crate::lint::Config::default(),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let schemas = doc.get("components").and_then(|c| c.get("schemas")).unwrap();
        let r = |name: &str| schemas.get(name).and_then(|s| s.get("$ref")).and_then(Value::as_str).map(str::to_owned);
        assert_eq!(schemas.as_object().unwrap().keys().collect::<Vec<_>>(), ["A", "B", "C", "Pet", "schemas-Pet"]);
        assert_eq!(r("A").as_deref(), Some("#/components/schemas/Pet"));
        assert_eq!(r("B").as_deref(), Some("#/components/schemas/schemas-Pet"));
        assert_eq!(schemas.get("schemas-Pet").and_then(|s| s.get("type")), Some(&Value::String("number".into())));
        let c_items = schemas.get("C").and_then(|c| c.get("items")).and_then(|i| i.get("$ref"));
        assert_eq!(c_items, Some(&Value::String("#/components/schemas/Pet".into())));
    }
}
