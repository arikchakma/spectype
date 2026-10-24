// Runs openapi-typescript 7.13.0's whole-document tests against the spectype npm package (Node API)
// and the native CLI (symlinked as `openapi-typescript`, so `--help` prints that name).
//
// Usage: node tasks/upstream_tests/run.mjs [vitest args]
//   OTS_DIR       an existing checkout's packages/openapi-typescript (default: clone into .ots/)
//   SPECTYPE_BIN  the CLI binary (default: target/release/spectype)
// Needs `npm ci` here and the native addon (npm run build --prefix npm).
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const TAG = "openapi-typescript@7.13.0";
const FILES = ["index", "node-api", "discriminators", "cli", "yaml", "invalid"];
const REDOCLY_SUITE = "Redocly config";

const here = import.meta.dirname;
const root = path.join(here, "../..");
const cache = path.join(here, ".ots");
const api = path.join(root, "npm/spectype/index.js");
const target = path.resolve(process.env.SPECTYPE_BIN ?? path.join(root, "target/release/spectype"));
const addon = fs.readdirSync(path.join(root, "npm/spectype")).find((f) => f.endsWith(".node")) ?? "spectype.node";

for (const [file, hint] of [
  [target, "cargo build --release"],
  [path.join(root, "npm/spectype", addon), "npm run build --prefix npm"],
  [path.join(here, "node_modules/vitest"), `npm ci --prefix ${path.relative(process.cwd(), here)}`],
]) {
  if (!fs.existsSync(file)) {
    console.error(`missing ${file}; run ${hint}`);
    process.exit(1);
  }
}

let pkg = process.env.OTS_DIR && path.resolve(process.env.OTS_DIR);
if (!pkg) {
  const checkout = path.join(cache, "repo");
  if (!fs.existsSync(checkout)) {
    const url = "https://github.com/openapi-ts/openapi-typescript.git";
    execFileSync("git", ["clone", "--quiet", "--depth=1", "--branch", TAG, url, checkout], { stdio: "inherit" });
  }
  pkg = path.join(checkout, "packages/openapi-typescript");
}
if (!fs.existsSync(path.join(pkg, "node_modules"))) fs.symlinkSync(path.join(here, "node_modules"), path.join(pkg, "node_modules"));

const bin = path.join(cache, "bin/openapi-typescript");
fs.mkdirSync(path.dirname(bin), { recursive: true });
fs.rmSync(bin, { force: true });
fs.symlinkSync(target, bin);

const rewrites = [
  [/"\.\.\/src\/index\.js"/g, JSON.stringify(api)],
  [/"(\.\/|\.\.\/\.\.\/\.\.\/)bin\/cli\.js"/g, JSON.stringify(bin)],
];
const tests = FILES.map((name) => {
  let text = fs.readFileSync(path.join(pkg, `test/${name}.test.ts`), "utf8");
  let hits = 0;
  for (const [from, to] of rewrites) text = text.replace(from, () => (hits++, to));
  if (!hits) throw new Error(`test/${name}.test.ts no longer imports ../src/index.js or bin/cli.js`);
  const out = `test/${name}.spectype.test.ts`;
  fs.writeFileSync(path.join(pkg, out), text);
  return out;
});

const report = path.join(cache, "results.json");
fs.rmSync(report, { force: true });
spawnSync(
  process.execPath,
  [
    path.join(here, "node_modules/vitest/vitest.mjs"),
    "run",
    ...tests,
    "--reporter=default",
    "--reporter=json",
    `--outputFile.json=${report}`,
    ...process.argv.slice(2),
  ],
  { cwd: pkg, stdio: "inherit" },
);
if (!fs.existsSync(report)) process.exit(1);

const results = JSON.parse(fs.readFileSync(report, "utf8")).testResults.flatMap((file) =>
  file.assertionResults.map((t) => ({ ...t, file: path.basename(file.name) })),
);
const groups = { core: [], redocly: [] };
for (const t of results) groups[t.ancestorTitles.includes(REDOCLY_SUITE) ? "redocly" : "core"].push(t);
const line = (label, list) => {
  const failed = list.filter((t) => t.status !== "passed");
  console.log(`${label}: ${list.length - failed.length}/${list.length} passed`);
  for (const t of failed) console.log(`  ✘ ${t.file} > ${[...t.ancestorTitles, t.title].join(" > ")}`);
  return failed.length;
};
console.log(`\n${TAG} whole-document tests against spectype`);
const failures = line("  Node API + CLI", groups.core);
line(`  ${REDOCLY_SUITE} (CLI redocly.yaml support)`, groups.redocly);
process.exit(failures ? 1 : 0);
