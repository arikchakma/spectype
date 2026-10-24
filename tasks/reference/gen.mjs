import { execFile } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(fileURLToPath(new URL("../..", import.meta.url)));
const GOLDEN = path.join(ROOT, "tests/golden");
const SELF = fileURLToPath(import.meta.url);

const BOOLEAN_FLAGS = [
  "additionalProperties",
  "alphabetize",
  "arrayLength",
  "contentNever",
  "defaultNonNullable",
  "propertiesRequiredByDefault",
  "emptyObjectsUnknown",
  "enum",
  "enumValues",
  "conditionalEnums",
  "dedupeEnums",
  "check",
  "excludeDeprecated",
  "exportType",
  "help",
  "immutable",
  "pathParamsAsTypes",
  "rootTypes",
  "rootTypesNoSchemaPrefix",
  "rootTypesKeepCasing",
  "makePathsEnum",
  "generatePathParams",
  "readWriteMarkers",
];

async function runCase({ input, flags: args }) {
  const { default: openapiTS, astToString, COMMENT_HEADER } = await import("openapi-typescript");
  const req = createRequire(fileURLToPath(import.meta.resolve("openapi-typescript")));
  const parser = req("yargs-parser");
  const { createConfig } = await import(pathToFileURL(req.resolve("@redocly/openapi-core")).href);
  const flags = parser(args, {
    boolean: BOOLEAN_FLAGS,
    string: ["output", "redocly"],
    alias: { redocly: ["c"], exportType: ["t"], output: ["o"] },
  });
  process.chdir(ROOT);
  const ast = await openapiTS(new URL(input, pathToFileURL(`${ROOT}/`)), {
    ...Object.fromEntries(BOOLEAN_FLAGS.filter((k) => k !== "check" && k !== "help").map((k) => [k, flags[k]])),
    redocly: await createConfig({}, { extends: ["minimal"] }),
    silent: true,
  });
  return `${COMMENT_HEADER}${astToString(ast)}`;
}

function scrub(message) {
  return `${message.split(ROOT).join("<root>").replace(/\x1b\[[0-9;]*m/g, "")}\n`;
}

if (process.argv[2] === "--case") {
  const c = JSON.parse(process.argv[3]);
  // openapi-typescript/redocly can call process.exit on fatal errors; surface those as case errors instead
  process.exit = (code) => {
    throw new Error(`process.exit(${code})`);
  };
  try {
    fs.writeFileSync(path.join(GOLDEN, `${c.name}.ts`), await runCase(c));
  } catch (err) {
    fs.writeFileSync(path.join(GOLDEN, `${c.name}.error`), scrub(err?.message ?? String(err)));
  }
} else {
  const only = process.argv.slice(2);
  const cases = JSON.parse(fs.readFileSync(path.join(ROOT, "tests/cases.json"), "utf8"))
    .filter((c) => only.length === 0 || only.includes(c.name))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  const names = new Set();
  for (const c of cases) {
    if (names.has(c.name) || !/^[A-Za-z0-9._+-]+$/.test(c.name)) throw new Error(`bad or duplicate case name: ${c.name}`);
    names.add(c.name);
  }
  fs.mkdirSync(GOLDEN, { recursive: true });
  if (only.length === 0) {
    for (const f of fs.readdirSync(GOLDEN)) fs.rmSync(path.join(GOLDEN, f));
  } else {
    for (const c of cases) for (const ext of [".ts", ".error"]) fs.rmSync(path.join(GOLDEN, c.name + ext), { force: true });
  }
  let next = 0;
  let failed = 0;
  // one process per case: openapi-typescript keeps module-level caches (e.g. enumCache) that would leak between cases
  const worker = async () => {
    while (next < cases.length) {
      const c = cases[next++];
      await new Promise((resolve) =>
        execFile(process.execPath, [SELF, "--case", JSON.stringify(c)], { maxBuffer: 1 << 28 }, (err, _out, stderr) => {
          if (err) {
            failed++;
            console.error(`worker crashed for ${c.name}: ${stderr || err.message}`);
          }
          resolve();
        }),
      );
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, os.availableParallelism() - 1) }, worker));
  const errors = cases.filter((c) => fs.existsSync(path.join(GOLDEN, `${c.name}.error`))).length;
  console.log(`${cases.length} cases, ${errors} .error, ${failed} crashed`);
  if (failed) process.exit(1);
}
