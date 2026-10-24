// Records what openapi-typescript 7.13.0 prints for fixtures/lint/cases.json: the lint lines
// (` ⚠  …` / ` ✘  …`) on stderr, the exit status, and the generated file when it succeeds.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(fileURLToPath(new URL("../..", import.meta.url)));
const CLI = path.join(ROOT, "tasks/reference/node_modules/openapi-typescript/bin/cli.js");
const OUT = path.join(ROOT, "fixtures/lint/expected");
const cases = JSON.parse(fs.readFileSync(path.join(ROOT, "fixtures/lint/cases.json"), "utf8"));
fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "spectype-lint-"));

for (const c of cases) {
  const out = path.join(tmp, "out.ts");
  fs.rmSync(out, { force: true });
  const args = [CLI, c.input, "--redocly", c.config, ...(c.stdout ? [] : ["-o", out])];
  const run = spawnSync(process.execPath, args, { cwd: ROOT, encoding: "utf8" });
  const status = run.status ?? 1;
  const { stdout, stderr } = run;
  const lines = stderr.replace(/\x1b\[[0-9;]*m/g, "").split("\n").filter((l) => /^ [⚠✘] {2}/.test(l));
  fs.writeFileSync(path.join(OUT, `${c.name}.txt`), `exit ${status}\n${lines.map((l) => `${l}\n`).join("")}`);
  if (!status) fs.writeFileSync(path.join(OUT, `${c.name}.ts`), c.stdout ? stdout : fs.readFileSync(out, "utf8"));
}
console.log(`${cases.length} cases recorded`);
