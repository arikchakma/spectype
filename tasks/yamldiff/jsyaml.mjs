// js-yaml as Redocly loads it, plus the canonical dump `yaml_internals::canonical` mirrors.

import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const yaml = createRequire(path.join(ROOT, "tasks/reference/package.json"))("js-yaml");

// Redocly's `parseYaml` schema; maxDepth matches spectype's nesting cap (js-yaml defaults to 100).
export const SCHEMA = yaml.JSON_SCHEMA.extend({
  implicit: [yaml.types.merge],
  explicit: [yaml.types.binary, yaml.types.omap, yaml.types.pairs, yaml.types.set],
});
export const load = (text) => yaml.load(text, { schema: SCHEMA, maxDepth: 1000 });

export class Unrepresentable extends Error {}

export function canon(v, stack = new Set()) {
  if (v === null || v === undefined) return "null";
  if (typeof v === "boolean") return String(v);
  if (typeof v === "number") {
    if (Object.is(v, -0)) return "-0";
    return String(v);
  }
  if (typeof v === "string") return quote(v);
  if (ArrayBuffer.isView(v)) throw new Unrepresentable("binary");
  if (stack.has(v)) throw new Unrepresentable("cycle");
  stack.add(v);
  let out;
  if (Array.isArray(v)) out = "[" + v.map((x) => canon(x, stack)).join(",") + "]";
  else out = "{" + Object.keys(v).map((k) => quote(k) + ":" + canon(v[k], stack)).join(",") + "}";
  stack.delete(v);
  return out;
}

function quote(s) {
  let out = '"';
  for (let i = 0; i < s.length; i++) {
    const u = s.charCodeAt(i);
    const cp = s.codePointAt(i);
    if (cp >= 0x10f000 && cp <= 0x10f7ff) throw new Unrepresentable("sentinel");
    if (u === 0x22) out += '\\"';
    else if (u === 0x5c) out += "\\\\";
    else if (u >= 0x20 && u <= 0x7e) out += s[i];
    else out += "\\u" + u.toString(16).padStart(4, "0");
  }
  return out + '"';
}

export function expected(text) {
  try {
    return "OK " + canon(load(text));
  } catch (e) {
    if (e instanceof Unrepresentable) return `UNREPRESENTABLE ${e.message}`;
    return "ERR " + String(e.message).split("\n")[0];
  }
}
