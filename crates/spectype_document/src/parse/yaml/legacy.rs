//! The yaml-rust2 event loader, kept for constructs the fast loader hands back (`!!binary`).

use rustc_hash::FxHashMap;

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use super::resolve_plain;
use crate::value::{Map, Value};
use crate::{Error, Result};

enum Frame {
    Seq(Vec<Value>, usize),
    Map { map: Map, overridable: Vec<String>, key: Option<Value>, anchor: usize },
}

#[derive(Default)]
struct Loader {
    stack: Vec<Frame>,
    anchors: FxHashMap<usize, Value>,
    root: Option<Value>,
    error: Option<String>,
    done: bool,
}

fn tagged(tag: &Tag, s: &str) -> Value {
    match (tag.handle.as_str(), tag.suffix.as_str()) {
        ("!!" | "tag:yaml.org,2002:", "null") => Value::Null,
        ("!!" | "tag:yaml.org,2002:", "bool" | "int" | "float") => resolve_plain(s),
        _ => Value::String(s.to_owned()),
    }
}

impl Loader {
    fn fail(&mut self, msg: String) {
        if self.error.is_none() {
            self.error = Some(msg);
        }
    }

    fn push_value(&mut self, v: Value, anchor: usize, mark: Marker) {
        if anchor > 0 {
            self.anchors.insert(anchor, v.clone());
        }
        match self.stack.last_mut() {
            None => {
                if self.root.is_none() {
                    self.root = Some(v);
                }
            }
            Some(Frame::Seq(items, _)) => items.push(v),
            Some(Frame::Map { map, overridable, key, .. }) => match key.take() {
                None => *key = Some(v),
                Some(k) => {
                    if matches!(&k, Value::String(s) if s == "<<") {
                        let sources = match v {
                            Value::Array(a) => a,
                            other => vec![other],
                        };
                        for source in sources {
                            if let Value::Object(m) = source {
                                for (mk, mv) in m.iter() {
                                    if !map.contains_key(mk) {
                                        map.insert(mk.clone(), mv.clone());
                                        overridable.push(mk.clone());
                                    }
                                }
                            }
                        }
                        return;
                    }
                    let k = match k {
                        Value::String(s) => s,
                        other => other.js_string(),
                    };
                    if map.contains_key(&k) && !overridable.contains(&k) {
                        let msg =
                            format!("duplicated mapping key \"{k}\" (line {}, column {})", mark.line(), mark.col() + 1);
                        self.fail(msg);
                        return;
                    }
                    overridable.retain(|o| o != &k);
                    map.insert(k, v);
                }
            },
        }
    }
}

impl MarkedEventReceiver for Loader {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.done || self.error.is_some() {
            return;
        }
        match ev {
            Event::DocumentEnd if self.root.is_some() => self.done = true,
            Event::Scalar(s, style, anchor, tag) => {
                let v = match (&tag, style) {
                    (Some(t), _) => tagged(t, &s),
                    (None, TScalarStyle::Plain) => resolve_plain(&s),
                    _ => Value::String(s),
                };
                self.push_value(v, anchor, mark);
            }
            Event::Alias(id) => match self.anchors.get(&id).cloned() {
                Some(v) => self.push_value(v, 0, mark),
                None => self.fail(format!("unidentified alias (line {})", mark.line())),
            },
            Event::SequenceStart(..) | Event::MappingStart(..) if self.stack.len() >= crate::parse::json::MAX_DEPTH => {
                self.fail(format!(
                    "nesting deeper than {} levels (line {})",
                    crate::parse::json::MAX_DEPTH,
                    mark.line()
                ));
            }
            Event::SequenceStart(anchor, _) => self.stack.push(Frame::Seq(Vec::new(), anchor)),
            Event::SequenceEnd => {
                if let Some(Frame::Seq(items, anchor)) = self.stack.pop() {
                    self.push_value(Value::Array(items), anchor, mark);
                }
            }
            Event::MappingStart(anchor, _) => {
                self.stack.push(Frame::Map { map: Map::new(), overridable: Vec::new(), key: None, anchor });
            }
            Event::MappingEnd => {
                if let Some(Frame::Map { map, anchor, .. }) = self.stack.pop() {
                    self.push_value(Value::Object(map), anchor, mark);
                }
            }
            _ => {}
        }
    }
}

pub fn parse(source: &str) -> Result<Value> {
    let mut loader = Loader::default();
    Parser::new_from_str(source).load(&mut loader, false).map_err(|e| Error::Parse(format!("invalid YAML: {e}")))?;
    if let Some(e) = loader.error {
        return Err(Error::Parse(format!("invalid YAML: {e}")));
    }
    Ok(loader.root.unwrap_or(Value::Null))
}
