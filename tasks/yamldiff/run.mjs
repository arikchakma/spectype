// Differential test: spectype's YAML loader vs js-yaml (Redocly's schema) on every YAML file in
// the repo, YAML dumped from every JSON fixture in many styles, and hand-written edge cases.
//
//   node tasks/yamldiff/run.mjs [--no-build] [--legacy] [--filter <substr>] [--show <n>]

import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import { adversarial } from "./cases.mjs";
import { SCHEMA, expected, yaml } from "./jsyaml.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const WORK = path.join(HERE, ".work");

const { values: opt } = parseArgs({
  options: {
    "no-build": { type: "boolean", default: false },
    legacy: { type: "boolean", default: false },
    filter: { type: "string" },
    show: { type: "string", default: "20" },
  },
});

function walk(dir, exts, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "node_modules" || e.name === ".work" || e.name === "target") continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, exts, out);
    else if (exts.some((x) => e.name.endsWith(x))) out.push(p);
  }
  return out;
}

function rng(seed) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32);
}

// Reuses finished subtrees so `dump` emits anchors and aliases.
function share(doc, seed) {
  const r = rng(seed);
  const seen = [];
  const visit = (v) => {
    if (v === null || typeof v !== "object") return v;
    const out = Array.isArray(v) ? [] : {};
    for (const k of Object.keys(v)) {
      if (seen.length > 0 && v[k] !== null && typeof v[k] === "object" && r() < 0.15) {
        out[k] = seen[Math.floor(r() * seen.length)];
      } else {
        out[k] = visit(v[k]);
      }
    }
    seen.push(out);
    return out;
  };
  return visit(doc);
}

const STYLES = [
  {},
  { flowLevel: 0 },
  { flowLevel: 1 },
  { flowLevel: 2, condenseFlow: true },
  { flowLevel: 3 },
  { indent: 1 },
  { indent: 4, noArrayIndent: true },
  { indent: 7, lineWidth: 30 },
  { lineWidth: 20 },
  { lineWidth: 40, indent: 3 },
  { lineWidth: -1 },
  { quotingType: '"' },
  { forceQuotes: true },
  { forceQuotes: true, quotingType: '"', lineWidth: 25 },
  { sortKeys: true, schema: SCHEMA },
  { condenseFlow: true, flowLevel: 1, quotingType: '"' },
  { noCompatMode: true, schema: SCHEMA },
  { schema: SCHEMA, lineWidth: 15, indent: 2 },
];

function collect() {
  const cases = [];
  const seen = new Set();
  const add = (source, text) => {
    const h = crypto.createHash("sha1").update(text).digest("hex");
    if (seen.has(h)) return;
    seen.add(h);
    cases.push({ source, text });
  };
  const yamlFiles = [
    ...walk(path.join(ROOT, "fixtures"), [".yaml", ".yml"]),
    ...walk(path.join(ROOT, "tests"), [".yaml", ".yml"]),
    ...walk(path.join(ROOT, "tasks/upstream_tests/.ots"), [".yaml", ".yml"]),
  ];
  for (const f of yamlFiles) add(path.relative(ROOT, f), fs.readFileSync(f, "utf8"));
  const jsonFiles = [...walk(path.join(ROOT, "fixtures"), [".json"]), ...walk(path.join(ROOT, "tests"), [".json"])];
  for (const f of jsonFiles) {
    const raw = fs.readFileSync(f, "utf8");
    let doc;
    try {
      doc = JSON.parse(raw);
    } catch {
      continue;
    }
    const rel = path.relative(ROOT, f);
    add(rel, raw);
    const styles = raw.length > 300_000 ? STYLES.slice(0, 3) : STYLES;
    styles.forEach((style, i) => {
      try {
        add(`${rel} dump#${i}`, yaml.dump(doc, style));
      } catch {}
    });
    try {
      add(`${rel} shared`, yaml.dump(share(doc, raw.length), { lineWidth: 50 }));
      add(`${rel} shared-flow`, yaml.dump(share(doc, raw.length + 1), { flowLevel: 2 }));
    } catch {}
  }
  adversarial().forEach((text, i) => add(`adversarial#${i}: ${JSON.stringify(text).slice(0, 60)}`, text));
  return cases;
}

let cases = collect();
if (opt.filter) cases = cases.filter((c) => c.source.includes(opt.filter));
fs.rmSync(WORK, { recursive: true, force: true });
fs.mkdirSync(path.join(WORK, "cases"), { recursive: true });
const list = cases.map((c, i) => {
  const p = path.join(WORK, "cases", `${i}.yaml`);
  fs.writeFileSync(p, c.text);
  return p;
});
fs.writeFileSync(path.join(WORK, "list.txt"), list.join("\n") + "\n");

if (!opt["no-build"]) {
  execFileSync("cargo", ["build", "--release", "-q", "-p", "spectype_document", "--example", "yaml_canon"], {
    cwd: ROOT,
    stdio: "inherit",
  });
}
const args = [path.join(WORK, "list.txt"), ...(opt.legacy ? ["--legacy"] : [])];
const actual = execFileSync(path.join(ROOT, "target/release/examples/yaml_canon"), args, {
  maxBuffer: 1 << 30,
  encoding: "utf8",
}).split("\n");

// Documented deviations: js-yaml hands out a half-built collection for an alias inside its own
// anchor (spectype cannot represent that and errors), and `!!binary` yields a Uint8Array (spectype
// defers to yaml-rust2, which keeps the base64 text). Strings holding U+10F000–U+10F7FF collide
// with how `Value` stores lone surrogates, so those cannot be compared at all.
const KNOWN = [
  (want, got, fellBack) => want.startsWith("OK") && got.includes("is not complete yet"),
  (want, got, fellBack) => want === "UNREPRESENTABLE binary" && fellBack,
  (want) => want === "UNREPRESENTABLE sentinel",
];

const tally = { same: 0, sameError: 0, known: 0, mismatch: 0, unrepresentable: 0, fallback: 0 };
const failures = [];
cases.forEach((c, i) => {
  let got = actual[i] ?? "";
  const fellBack = got.startsWith("FALLBACK ");
  if (fellBack) {
    tally.fallback++;
    got = got.slice("FALLBACK ".length);
  }
  const want = expected(c.text);
  if (KNOWN.some((k) => k(want, got, fellBack))) tally.known++;
  else if (want.startsWith("UNREPRESENTABLE")) {
    tally.unrepresentable++;
    if (!got.startsWith("ERR")) failures.push({ c, i, want, got });
  } else if (want.startsWith("ERR") && got.startsWith("ERR")) tally.sameError++;
  else if (want === got) tally.same++;
  else failures.push({ c, i, want, got });
});
tally.mismatch = failures.length;

const show = Number(opt.show);
for (const { c, i, want, got } of failures.slice(0, show)) {
  let at = 0;
  while (at < want.length && want[at] === got[at]) at++;
  console.log(`\n✗ ${c.source}  (.work/cases/${i}.yaml)`);
  console.log(`  js-yaml : ${want.slice(Math.max(0, at - 60), at + 100)}`);
  console.log(`  spectype: ${got.slice(Math.max(0, at - 60), at + 100)}`);
}
console.log(`\n${cases.length} cases:`, tally);
process.exitCode = failures.length ? 1 : 0;
