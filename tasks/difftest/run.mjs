import { execFile, execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const WORK = path.join(HERE, ".work");
const FAILURES = path.join(HERE, "failures");
const UPSTREAM = path.join(ROOT, "tasks/reference/node_modules/openapi-typescript/bin/cli.js");
const SPECTYPE = path.join(ROOT, "target/release/spectype");

const { values: opt } = parseArgs({
  options: {
    seed: { type: "string", default: "1" },
    count: { type: "string", default: "200" },
    concurrency: { type: "string", default: "8" },
    "no-build": { type: "boolean", default: false },
    dump: { type: "string" },
    "max-steps": { type: "string", default: "800" },
  },
});
const SEED = Number(opt.seed);
const COUNT = Number(opt.count);
const CONCURRENCY = Number(opt.concurrency);

const PRIMARY = ["--enum-values", "--root-types", "--root-types-no-schema-prefix"];
const EXTRA = [
  "--enum",
  "--dedupe-enums",
  "--conditional-enums",
  "--export-type",
  "--immutable",
  "--additional-properties",
  "--empty-objects-unknown",
  "--default-non-nullable=false",
  "--properties-required-by-default",
  "--array-length",
  "--path-params-as-types",
  "--alphabetize",
  "--exclude-deprecated",
  "--make-paths-enum",
  "--generate-path-params",
  "--read-write-markers",
  "--root-types-keep-casing",
];

const COMPONENT_NAMES = ["Pet", "pet", "Pet_Owner", "pet-owner", "Foo.Bar", "200", "Ünïcode", "Error", "ERROR", "_under", "Schema", "Record", "Date", "enum", "class", "123abc", "ApiResponse", "user_v2", "a$b", "Paths", "with space", "a/b", "x~y", "ResponsePet", "ParameterPet", "RequestBodyPet", "HeaderPet", "PathItemPet", "SchemaPet", "pet_owner"];
const PROP_NAMES = ["id", "name", "type", 'say "hi"', "it's", "ünï", "123", "200", "1.5", "-", "kebab-case", "snake_case", "$dollar", "", "with space", "a.b", "camelCase", "_", "0", "petType", "kind", "back\\slash", "new\nline", "emoji🎉"];
const TEXTS = ["Simple text", "line1\nline2", "has */ inside", "tab\there", "trailing  ", "", "quote \" and '", "back\\slash", "emoji 🎉", "<b>html</b>", "@deprecated tag", "multi\n\nparagraph", " leading", "ends with newline\n", "/* open", "*/", "a\r\nb", "`tick`", "${x}", "nbsp\u00a0here", "ls\u2028sep", "bell\u0007", "\ufeffbom", "**bold** _it_", "@example x", "end */ start /*"];
const STRINGS = ["a", "B", "with space", "", 'quo"te', "it's", "123", "1.5", "-", "true", "null", "ünï", "a-b", "$x", "new\nline", "back\\slash", "*/", "tab\t", "UPPER_CASE", "camelCase", "0", "-1", "ls\u2028", "nbsp\u00a0", "1e3", "0x1F", "  padded  ", "a.b", "a/b", "#hash", "constructor"];
const NUMBERS = [0, 1, -1, 1.5, 100, 0.1, -0.5, 1e21, 2147483648, 9007199254740991, 42];
const PARAM_NAMES = ["id", "limit", "X-Request-ID", "q", "sort[by]", "page.size", "user-id", "$top", "ünï", "cookieId", "petId", "ext", "Accept", "tags"];
const SEGMENTS = ["pets", "users", "{id}", "{petId}", "v1", "items.{ext}", "{user-id}", "store", "a b", "{ünï}", "x:y", "", "{id}.json"];
const MEDIA = ["application/json", "application/json", "text/plain", "application/xml", "*/*", "application/vnd.api+json", "multipart/form-data", "application/octet-stream", "text/html; charset=utf-8"];
const CODES = ["200", "200", "201", "204", "2XX", "4XX", "default", "default", "404", "500", "1XX", "302"];
const OPERATION_IDS = ["a%20b", "get%2Fpets", "getPets", "get#pets", "a/b", "list pets", "123", "getPets", "createPet", "update-pet", "ünï", "deletePet"];
const METHODS = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];
const FORMATS = ["date-time", "date", "binary", "byte", "int64", "int32", "uuid", "email", "float", "double", "weird"];
const DEEP = "\0deep";
// JS numbers can't carry their source spelling, so these are spliced into the JSON text verbatim
const RAW = ["1.0", "1e2", "-0", "1E+2", "0.10", "100000000000000000000000", "12345678901234567890", "1e400", "5e-324", "0.1e1", "-0.0", "1e-7", "123.456e5", '"a\\/b"', '"\\u00e9"', '"\\ud83d\\ude00"'].map((r) => `\0raw:${r}`);
const serialize = (v) => JSON.stringify(v, null, 2).replace(/"\\u0000raw:((?:[^"\\]|\\.)*)"/g, (_, r) => JSON.parse(`"${r}"`));

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function generate(r) {
  const int = (n) => Math.floor(r() * n);
  const pick = (a) => a[int(a.length)];
  const p = (x) => r() < x;
  const range = (lo, hi) => Array.from({ length: lo + int(hi - lo + 1) }, (_, i) => i);
  const wpick = (choices) => {
    let x = r() * choices.reduce((s, [w]) => s + w, 0);
    for (const [w, f] of choices) if ((x -= w) < 0) return f();
    return choices.at(-1)[1]();
  };
  const uniq = (pool, n) => [...new Set(range(0, n).map(() => pick(pool)))];
  const esc = (s) => s.replace(/~/g, "~0").replace(/\//g, "~1");

  const v31 = p(0.5);
  const names = {
    schemas: uniq(COMPONENT_NAMES, 10),
    parameters: p(0.4) ? uniq(COMPONENT_NAMES, 3) : [],
    responses: p(0.4) ? uniq(COMPONENT_NAMES, 3) : [],
    requestBodies: p(0.3) ? uniq(COMPONENT_NAMES, 2) : [],
    headers: p(0.3) ? uniq(COMPONENT_NAMES, 2) : [],
    pathItems: v31 && p(0.25) ? uniq(COMPONENT_NAMES, 2) : [],
  };
  const groups = p(0.4) ? range(1, 2).map(() => ({ base: `${pick(["Animal", "Shape", "Event"])}${int(9)}`, kids: uniq(["Cat", "Dog", "Lizard", "circle", "Square-Thing", "Pet"], 3) })) : [];
  for (const g of groups) {
    g.kids = g.kids.filter((k) => k !== g.base);
    names.schemas = [...new Set([...names.schemas, g.base, ...g.kids])];
  }
  const ref = (section, name = pick(names[section])) => ({ $ref: `#/components/${section}/${p(0.05) ? encodeURIComponent(esc(name)) : esc(name)}` });
  const maybe = (x, f) => (p(x) ? f() : undefined);

  const json = (d = 0) =>
    wpick([
      [3, () => pick(STRINGS)],
      [2, () => pick(NUMBERS)],
      [1, () => pick(RAW)],
      [1, () => null],
      [1, () => p(0.5)],
      [d < 2 ? 1 : 0, () => Object.fromEntries(range(0, 2).map(() => [pick(PROP_NAMES), json(d + 1)]))],
      [d < 2 ? 1 : 0, () => range(0, 2).map(() => json(d + 1))],
    ]);

  const enumSchema = () => {
    const kind = pick(["string", "string", "number", "mixed", "objects"]);
    const value = { string: () => pick(STRINGS), number: () => (p(0.2) ? pick(RAW.slice(0, 13)) : pick(NUMBERS)), mixed: () => json(1), objects: () => json(1) }[kind];
    const values = p(0.04) ? [] : [...new Set(range(1, 5).map(value))];
    if (p(0.15)) values.push(null);
    if (p(0.08)) values.push(p(0.5));
    const s = { enum: values };
    if (kind === "string" && p(0.7)) s.type = "string";
    if (kind === "number" && p(0.7)) s.type = pick(["integer", "number"]);
    if (values.includes(null) && p(0.5)) v31 && s.type ? (s.type = [s.type, "null"]) : (s.nullable = true);
    const n = () => values.length - (p(0.2) ? 1 : 0);
    if (p(0.15)) s["x-enum-varnames"] = range(n(), n()).map((i) => pick(["Foo", "BAR", `Value${i}`, "with space", "1st", "ünï", ""]));
    if (p(0.08)) s["x-enumNames"] = range(n(), n()).map((i) => `Name${i}`);
    if (p(0.12)) s["x-enum-descriptions"] = range(n(), n()).map(() => pick(TEXTS));
    return s;
  };

  const objectSchema = (d) => {
    const s = {};
    if (p(0.85)) s.type = "object";
    const props = uniq(p(0.01) ? [...PROP_NAMES, "constructor", "toString"] : PROP_NAMES, p(0.1) ? 0 : 6);
    if (props.length || p(0.3)) s.properties = Object.fromEntries(props.map((k) => [k, schema(d + 1)]));
    if (p(0.6)) {
      const req = props.filter(() => p(0.5));
      if (p(0.1)) req.push(pick(["missing", "id", "nope"]));
      s.required = p(0.01) ? pick([true, "id", {}]) : req;
    }
    if (p(0.3)) s.additionalProperties = pick([true, false, {}, () => schema(d + 1), { type: "string" }]);
    if (typeof s.additionalProperties === "function") s.additionalProperties = s.additionalProperties();
    if (p(0.08)) s.patternProperties = { "^x-": schema(d + 1), ...(p(0.3) ? { "\\d+": schema(d + 1) } : {}) };
    if (p(0.06)) s.$defs = { Inner: schema(d + 1), ...(p(0.3) ? { "other-def": schema(d + 1) } : {}) };
    if (p(0.04)) s.minProperties = 1;
    return s;
  };

  const arraySchema = (d) => {
    const s = p(0.9) ? { type: "array" } : {};
    wpick([
      [6, () => (s.items = schema(d + 1))],
      [1, () => (s.items = range(1, 3).map(() => schema(d + 1)))],
      [2, () => ((s.prefixItems = range(1, 3).map(() => schema(d + 1))), p(0.5) && (s.items = pick([false, schema(d + 1)])))],
      [1, () => {}],
    ]);
    if (p(0.3)) s.minItems = pick([0, 1, 2, 3, 30]);
    if (p(0.25)) s.maxItems = pick([0, 1, 2, 5, 100]);
    if (p(0.05)) s.uniqueItems = true;
    return s;
  };

  const composition = (d) => {
    const part = () =>
      wpick([
        [5, () => (names.schemas.length ? withSiblings(ref("schemas")) : schema(d + 1))],
        [5, () => schema(d + 1)],
        [1, () => ({ required: uniq(PROP_NAMES, 2) })],
        [1, () => ({ type: "null" })],
        [1, () => ({ properties: { [pick(PROP_NAMES)]: { readOnly: true, type: "string" } }, required: [pick(PROP_NAMES)] })],
      ]);
    const parts = () => range(p(0.1) ? 0 : 1, 3).map(part);
    const s = {};
    for (const k of uniq(["allOf", "anyOf", "oneOf"], p(0.8) ? 0 : 1)) s[k] = parts();
    if (p(0.2)) Object.assign(s, objectSchema(d));
    else if (p(0.1)) s.type = pick(["object", "string", "null"]);
    if (p(0.1) && groups.length) {
      const g = pick(groups);
      s.oneOf = g.kids.map((k) => ref("schemas", k));
      s.discriminator = { propertyName: "petType", ...(p(0.5) ? { mapping: Object.fromEntries(g.kids.map((k) => [k.toLowerCase(), ref("schemas", k).$ref])) } : {}) };
    }
    return s;
  };

  const withSiblings = (s) => {
    if (p(0.15)) s.description = pick(TEXTS);
    if (p(0.06)) s.nullable = true;
    if (p(0.04)) s.deprecated = true;
    if (p(0.04)) s.readOnly = true;
    if (p(0.03)) s.default = json(1);
    if (p(0.03)) s.type = pick(["object", "string"]);
    return s;
  };

  const decorate = (s) => {
    if (p(0.15)) s.description = pick(TEXTS);
    if (p(0.08)) s.title = pick(TEXTS);
    if (p(0.05)) s.summary = pick(TEXTS);
    if (p(0.08)) s.deprecated = p(0.85);
    if (p(0.07)) s.readOnly = true;
    if (p(0.06)) s.writeOnly = true;
    if (p(0.12)) s.default = s.enum && s.enum.length && p(0.7) ? pick(s.enum) : json(1);
    if (p(0.06)) s.example = json(1);
    if (p(0.04)) s.examples = range(1, 2).map(() => json(1));
    if (p(v31 ? 0.04 : 0.15) && !s.$ref) s.nullable = p(0.8);
    if (p(0.04)) s.const = json(1);
    if (p(0.0005)) s[pick(["example", "description"])] = { $ref: DEEP };
    if (p(0.05)) Object.assign(s, pick([{ "x-foo": json(1) }, { $comment: "c" }, { $id: "x" }, { contentMediaType: "image/png" }, { unevaluatedProperties: false }, { propertyNames: { pattern: "^a" } }, { if: { type: "string" }, then: { minLength: 1 } }, { contains: { type: "string" } }, { "x-nullable": true }, { dependentRequired: { a: ["b"] } }, { items: true }]));
    return s;
  };

  const schema = (d = 0) => {
    if (d > 4 || p(0.07 * d)) return decorate({ type: pick(["string", "integer", "number", "boolean"]) });
    const s = wpick([
      [4, () => {
        // top-level pure aliases only point backwards, so alias chains never cycle (redocly hard-errors on those)
        const pool = d === 0 ? names.schemas.slice(0, current) : names.schemas;
        return pool.length ? withSiblings(ref("schemas", pick(pool))) : { type: "string" };
      }],
      [0.5, () => ({ $ref: DEEP })],
      [4, () => ({ type: pick(["string", "string", "integer", "number", "boolean", "null"]), ...(p(0.3) ? { format: pick(FORMATS) } : {}) })],
      [5, () => objectSchema(d)],
      [3, () => arraySchema(d)],
      [3, enumSchema],
      [2, () => ({ const: json(1), ...(p(0.3) ? { type: pick(["string", "number", "boolean", "object"]) } : {}) })],
      [3, () => composition(d)],
      [v31 ? 2 : 0.3, () => {
        const types = uniq(["string", "null", "number", "integer", "boolean", "object", "array"], 3);
        const s = { type: p(0.05) ? [] : types };
        if (types.includes("object") && p(0.6)) Object.assign(s, objectSchema(d), { type: types });
        if (types.includes("array") && p(0.6)) s.items = schema(d + 1);
        return s;
      }],
      [1, () => ({ type: pick(["file", "any", "String", "int", "date"]) })],
      [1, () => ({})],
      [v31 ? 0.5 : 0, () => p(0.5)],
      [0.5, () => ({ not: schema(d + 1) })],
    ]);
    return typeof s === "object" && !s.$ref ? decorate(s) : s;
  };

  const content = () =>
    Object.fromEntries(
      uniq(MEDIA, 2).map((m) => {
        const media = p(0.9) ? { schema: schema(1) } : {};
        if (p(0.08)) media.example = json(1);
        if (p(0.05)) media.examples = { one: { value: json(1) } };
        return [m, media];
      }),
    );

  const param = (loc, name) => {
    const s = { name, in: loc };
    if (p(0.03)) delete s.name;
    if (loc === "path" ? p(0.9) : p(0.4)) s.required = loc === "path" ? p(0.95) : p(0.6);
    if (p(0.93)) s.schema = p(0.3) ? enumSchema() : schema(2);
    else if (p(0.5)) s.content = { "application/json": { schema: schema(2) } };
    if (p(0.15)) s.description = pick(TEXTS);
    if (p(0.08)) s.deprecated = true;
    if (p(0.05)) s.example = json(1);
    return s;
  };
  const anyParam = () => param(pick(["query", "query", "header", "cookie", "path"]), pick(PARAM_NAMES));
  const params = (pathParams) => {
    const list = pathParams.filter(() => p(0.6)).map((n) => param("path", n));
    for (const _ of range(0, 2)) list.push(names.parameters.length && p(0.4) ? ref("parameters") : anyParam());
    return list;
  };

  const header = () => {
    const h = p(0.9) ? { schema: schema(2) } : { content: { "text/plain": { schema: { type: "string" } } } };
    if (p(0.4)) h.required = p(0.7);
    if (p(0.15)) h.description = pick(TEXTS);
    if (p(0.05)) h.deprecated = true;
    return h;
  };
  const response = () => {
    if (names.responses.length && p(0.25)) return ref("responses");
    const res = { description: pick(TEXTS) };
    if (p(0.3)) res.headers = Object.fromEntries(uniq(["X-Rate-Limit", "x-next", "Set-Cookie", "ünï", "Content-Type"], 2).map((h) => [h, names.headers.length && p(0.3) ? ref("headers") : header()]));
    if (p(0.8)) res.content = content();
    return res;
  };
  const requestBody = () => {
    if (names.requestBodies.length && p(0.3)) return ref("requestBodies");
    const b = { content: p(0.04) ? {} : content() };
    if (p(0.6)) b.required = p(0.7);
    if (p(0.2)) b.description = pick(TEXTS);
    return b;
  };
  const operation = (pathParams) => {
    const op = {};
    if (p(0.7)) op.operationId = pick(OPERATION_IDS);
    if (p(0.3)) op.summary = pick(TEXTS);
    if (p(0.2)) op.description = pick(TEXTS);
    if (p(0.08)) op.deprecated = true;
    if (p(0.2)) op.tags = [pick(["pets", "store"])];
    if (p(0.6)) op.parameters = params(pathParams);
    if (p(0.35)) op.requestBody = requestBody();
    if (p(0.08)) op.callbacks = { onEvent: { "{$request.body#/url}": { post: { requestBody: requestBody(), responses: { 200: { description: "ok" } } } } } };
    if (p(0.97)) op.responses = Object.fromEntries(uniq(CODES, 2).map((c) => [c, response()]));
    return op;
  };
  const pathItem = (pathParams = []) => {
    const item = {};
    if (p(0.1)) item.summary = pick(TEXTS);
    if (p(0.1)) item.description = pick(TEXTS);
    if (p(0.35)) item.parameters = params(pathParams);
    for (const m of uniq(METHODS, 2)) item[m] = operation(pathParams);
    return item;
  };

  let current = Infinity;
  const doc = { openapi: v31 ? pick(["3.1.0", "3.1.1"]) : pick(["3.0.0", "3.0.3"]), info: { title: "fuzz", version: "1" } };
  if (p(0.1)) doc.info.description = pick(TEXTS);
  doc.paths = {};
  for (const _ of range(0, 4)) {
    const url = `/${range(1, 3).map(() => pick(SEGMENTS)).join("/")}`;
    const pathParams = [...url.matchAll(/\{([^}]*)\}/g)].map((m) => m[1]);
    doc.paths[url] = names.pathItems.length && p(0.25) ? ref("pathItems") : pathItem(pathParams);
  }
  if (p(v31 ? 0.3 : 0.05)) doc.webhooks = Object.fromEntries(uniq(["newPet", "pet.updated", "ünï"], 2).map((w) => [w, pathItem()]));

  const components = {};
  if (names.schemas.length) components.schemas = Object.fromEntries(names.schemas.map((n, i) => [n, ((current = i), schema(0))]));
  current = Infinity;
  for (const g of groups) {
    const mapping = p(0.5) ? Object.fromEntries(g.kids.filter(() => p(0.8)).map((k) => [pick([k, k.toLowerCase(), "dog-1", "1"]), ref("schemas", k).$ref])) : undefined;
    const tag = p(0.5) ? { type: "string" } : enumSchema();
    if (p(0.5)) {
      components.schemas[g.base] = decorate({ type: "object", required: ["petType"], properties: { petType: tag, ...objectSchema(2).properties }, discriminator: { propertyName: "petType", ...(mapping && { mapping }) } });
      for (const k of g.kids) components.schemas[k] = { allOf: [ref("schemas", g.base), objectSchema(2)], ...(p(0.2) ? { description: pick(TEXTS) } : {}) };
    } else {
      components.schemas[g.base] = { oneOf: g.kids.map((k) => ref("schemas", k)), discriminator: { propertyName: "petType", ...(mapping && { mapping }) } };
      for (const k of g.kids) {
        const kid = objectSchema(2);
        kid.properties = { ...kid.properties, petType: p(0.5) ? { type: "string", enum: [k] } : tag };
        if (p(0.6)) kid.required = ["petType"];
        components.schemas[k] = kid;
      }
    }
  }
  for (const n of names.parameters) (components.parameters ??= {})[n] = anyParam();
  for (const n of names.responses) (components.responses ??= {})[n] = { description: pick(TEXTS), ...(p(0.8) ? { content: content() } : {}), ...(p(0.2) ? { headers: { "X-H": header() } } : {}) };
  for (const n of names.requestBodies) (components.requestBodies ??= {})[n] = { content: content(), ...(p(0.5) ? { required: p(0.5) } : {}) };
  for (const n of names.headers) (components.headers ??= {})[n] = header();
  for (const n of names.pathItems) (components.pathItems ??= {})[n] = pathItem();
  if (Object.keys(components).length || p(0.5)) doc.components = components;

  const targets = [];
  const walk = (node, ptr, keys) => {
    if (!node || typeof node !== "object" || node.$ref) return;
    const [k, pk, gk] = keys;
    // only non-ref schemas are targets: ref-to-ref chains through deep pointers can cycle, which redocly hard-errors on
    if (!Array.isArray(node) && (["schema", "items", "additionalProperties", "not"].includes(k) || ["properties", "$defs", "patternProperties"].includes(pk) || (pk === "schemas" && gk === "components") || (k === "properties" && p(0.01)))) targets.push(ptr);
    for (const [ck, v] of Object.entries(node)) walk(v, `${ptr}/${esc(ck)}`, [ck, ...keys]);
  };
  walk(doc, "#", []);
  const lost = p(0.02);
  if (lost) targets.length = 0;
  const resolve = (node, ptr) => {
    if (!node || typeof node !== "object") return;
    for (const [k, v] of Object.entries(node)) resolve(v, `${ptr}/${esc(k)}`);
    // a ref to its own location is a redocly hard error that would hide everything else in the doc
    const ok = targets.filter((t) => t !== ptr);
    if (node.$ref !== DEEP) return;
    if (lost) node.$ref = "#/components/schemas/Missing";
    else if (ok.length) node.$ref = pick(ok);
    else delete node.$ref, (node.type = "string");
  };
  resolve(doc, "#");

  const flags = wpick([
    [4, () => [...PRIMARY, ...EXTRA.filter(() => p(0.1))]],
    [3, () => [...PRIMARY, ...EXTRA.filter(() => p(0.35))]],
    [3, () => [...PRIMARY, ...EXTRA].filter(() => p(0.25))],
  ]);
  return { spec: doc, flags };
}

let active = 0;
const waiting = [];
async function slot(fn) {
  while (active >= CONCURRENCY) await new Promise((res) => waiting.push(res));
  active++;
  try {
    return await fn();
  } finally {
    active--;
    waiting.shift()?.();
  }
}

const exec = (cmd, args) =>
  new Promise((res) =>
    execFile(cmd, args, { cwd: WORK, maxBuffer: 1 << 28, timeout: 120_000 }, (err, stdout, stderr) => res({ ok: !err, stdout, stderr: String(stderr || err?.message || "") })),
  );

let fileCounter = 0;
const firstLine = (s) => {
  const lines = s.replace(/\x1b\[[0-9;]*m/g, "").split("\n").map((l) => l.trim());
  return lines.find((l) => /^(\w*Error\b|✘|error)/i.test(l)) ?? lines.find(Boolean) ?? "";
};
const normalize = (s) => s.replace(/ at #\/.*/, " at <pointer>").replace(new RegExp(WORK.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "/\\S*", "g"), "<file>").replace(/"[^"]*"|'[^']*'|`[^`]*`/g, '"…"').replace(/\d+/g, "N");

function diff(expected, actual) {
  const a = expected.split("\n");
  const b = actual.split("\n");
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  let j = 0;
  while (j < a.length - i && j < b.length - i && a[a.length - 1 - j] === b[b.length - 1 - j]) j++;
  return { line: i + 1, expected: a.slice(i, a.length - j).slice(0, 25), actual: b.slice(i, b.length - j).slice(0, 25) };
}

async function check(spec, flags) {
  return slot(async () => {
    const file = path.join(WORK, `${process.pid}-${fileCounter++}.json`);
    fs.writeFileSync(file, serialize(spec));
    const [u, s] = await Promise.all([exec(process.execPath, [UPSTREAM, file, ...flags]), exec(SPECTYPE, [file, ...flags])]);
    fs.rmSync(file, { force: true });
    if (!u.ok) return { kind: "upstream-error", sig: normalize(firstLine(u.stderr)), spectypeErrored: !s.ok, u, s };
    if (!s.ok) return { kind: "spectype-error", sig: `spectype-error: ${normalize(firstLine(s.stderr))}`, u, s };
    if (u.stdout === s.stdout) return { kind: "identical", u, s };
    const d = diff(u.stdout, s.stdout);
    return { kind: "mismatch", sig: normalize(`${d.expected[0]?.trim()}\n${d.actual[0]?.trim()}`), u, s };
  });
}

function paths(root) {
  const out = [];
  const queue = [[root, []]];
  while (queue.length) {
    const [node, at] = queue.shift();
    if (at.length) out.push(at);
    if (node && typeof node === "object") for (const k of Object.keys(node)) queue.push([node[k], [...at, Array.isArray(node) ? Number(k) : k]]);
  }
  return out;
}

function variants(spec, at) {
  const clone = () => structuredClone(spec);
  const parentOf = (s) => at.slice(0, -1).reduce((n, k) => n[k], s);
  const key = at.at(-1);
  const value = parentOf(spec)[key];
  const out = [];
  if (!(at.length <= 2 && ["openapi", "info", "title", "version"].includes(key))) {
    const s = clone();
    const parent = parentOf(s);
    Array.isArray(parent) ? parent.splice(key, 1) : delete parent[key];
    out.push(s);
  }
  const replace = (v) => {
    const s = clone();
    parentOf(s)[key] = v;
    out.push(s);
  };
  if (value && typeof value === "object" && Object.keys(value).length > 1) {
    replace(Array.isArray(value) ? [] : {});
    for (const child of Object.values(value)) if (child && typeof child === "object" && !Array.isArray(value) === !Array.isArray(child)) replace(child);
  }
  if (typeof value === "string" && value.length > 1 && key !== "$ref" && key !== "openapi") replace("a");
  return out;
}

async function minimize(start, original) {
  let best = start;
  let steps = 0;
  const same = (r) => r.kind === original.kind;
  const budget = Number(opt["max-steps"]);
  for (let changed = true; changed && steps < budget; ) {
    changed = false;
    for (let i = 0; i < best.flags.length && steps < budget; i++) {
      const flags = best.flags.filter((_, j) => j !== i);
      steps++;
      if (same(await check(best.spec, flags))) (best = { ...best, flags }), i--, (changed = true);
    }
    const all = paths(best.spec);
    for (let i = 0; i < all.length && steps < budget; i++) {
      const cands = variants(best.spec, all[i]);
      steps += cands.length;
      const results = await Promise.all(cands.map((spec) => check(spec, best.flags)));
      const hit = results.findIndex(same);
      if (hit < 0) continue;
      best = { ...best, spec: cands[hit] };
      changed = true;
      all.splice(0, all.length, ...paths(best.spec));
      i--;
    }
  }
  return best;
}

const stats = { run: 0, identical: 0, mismatch: 0, "spectype-error": 0, "upstream-error": 0, upstreamErrorBothErrored: 0 };
const seen = new Map();
const upstreamErrors = new Map();

async function runCase(index) {
  const c = generate(rng(SEED * 1_000_003 + index));
  const res = await check(c.spec, c.flags);
  stats.run++;
  stats[res.kind]++;
  if (res.kind === "upstream-error") {
    if (res.spectypeErrored) stats.upstreamErrorBothErrored++;
    const k = `${res.sig}${res.spectypeErrored ? "" : "   [spectype succeeded]"}`;
    upstreamErrors.set(k, (upstreamErrors.get(k) ?? 0) + 1);
  }
  if (res.kind !== "mismatch" && res.kind !== "spectype-error") return;
  if (seen.has(res.sig)) return void seen.get(res.sig).hits++;
  const entry = { hits: 1 };
  seen.set(res.sig, entry);
  const min = await minimize(c, res);
  const final = await check(min.spec, min.flags);
  if (final.kind !== res.kind) return;
  const sig = final.sig;
  const hash = crypto.createHash("sha1").update(sig).digest("hex").slice(0, 8);
  entry.hash = hash;
  const out = path.join(FAILURES, `${hash}.json`);
  if (fs.existsSync(out)) return;
  fs.writeFileSync(
    out,
    serialize(
      { kind: final.kind, origin: { seed: SEED, index }, flags: min.flags, spec: min.spec, diff: final.kind === "mismatch" ? diff(final.u.stdout, final.s.stdout) : final.s.stderr, upstream: final.u.stdout, spectype: final.s.stdout },
    ),
  );
  console.log(`[${final.kind}] ${hash} (case ${index}, flags ${min.flags.join(" ") || "-"})\n  - ${sig.split("\n").join("\n  + ")}`);
}

if (opt.dump) {
  console.log(serialize(generate(rng(SEED * 1_000_003 + Number(opt.dump)))));
  process.exit(0);
}
if (!opt["no-build"]) execFileSync("cargo", ["build", "--release", "--quiet"], { cwd: ROOT, stdio: "inherit" });
fs.mkdirSync(WORK, { recursive: true });
fs.mkdirSync(FAILURES, { recursive: true });
await Promise.all(Array.from({ length: COUNT }, (_, i) => runCase(i)));

console.log(`\nseed ${SEED}: ${stats.run} cases, ${stats.identical} identical, ${stats.mismatch} mismatches (${[...seen].filter(([s]) => !s.startsWith("spectype-error")).length} raw signatures), ${stats["upstream-error"]} upstream errors (${stats.upstreamErrorBothErrored} also spectype errored), ${stats["spectype-error"]} spectype errors where upstream succeeded`);
for (const [sig, e] of seen) console.log(`  ${String(e.hits).padStart(4)}x ${e.hash ?? "--------"} ${sig.replace(/\n/g, "  ⇒  ")}`);
if (upstreamErrors.size) console.log("upstream errors:");
for (const [sig, n] of [...upstreamErrors].sort((a, b) => b[1] - a[1])) console.log(`  ${String(n).padStart(4)}x ${sig}`);
