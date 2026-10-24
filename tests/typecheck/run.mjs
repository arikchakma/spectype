import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const bin = process.env.SPECTYPE_BIN ?? join(root, "target/release/spectype");
const primary = ["--enum-values", "--root-types", "--root-types-no-schema-prefix"];

const specs = {
  fetch: "fixtures/fetch/openapi.json",
  github: "fixtures/ots/examples/github-api.yaml",
  "github-next": "fixtures/ots/examples/github-api-next.yaml",
  stripe: "fixtures/ots/examples/stripe-api.yaml",
  octokit: "fixtures/ots/examples/octokit-ghes-3.6-diff-to-api.json",
  simple: "fixtures/ots/examples/simple-example.yaml",
  "enum-root-types": "fixtures/ots/examples/enum-root-types.yaml",
};
const flagSets = {
  default: [],
  immutable: ["--immutable"],
  "export-type": ["--export-type", "--immutable"],
  required: ["--properties-required-by-default"],
  "empty-unknown": ["--empty-objects-unknown", "--additional-properties"],
  "root-types": ["--root-types"],
  fixed: [...primary, "--fix=all"],
  "fixed-enum": ["--enum", "--root-types", "--root-types-no-schema-prefix", "--fix=all"],
  "fixed-keep-casing": ["--root-types", "--root-types-keep-casing", "--root-types-no-schema-prefix", "--fix=all"],
};
// Upstream's own output for these does not compile on large specs (see COMPAT.md), so they are
// only type-checked where openapi-typescript gets them right.
const narrowFlagSets = {
  primary: { flags: primary, specs: ["fetch", "simple", "enum-root-types"] },
};

const out = join(here, "generated");
rmSync(out, { recursive: true, force: true });
mkdirSync(out);
const generate = (name, set, flags) => {
  const code = execFileSync(bin, [join(root, specs[name]), ...flags], { maxBuffer: 1 << 30 });
  writeFileSync(join(out, `${name}.${set}.ts`), code);
};
for (const name of Object.keys(specs)) {
  for (const [set, flags] of Object.entries(flagSets)) generate(name, set, flags);
}
for (const [set, { flags, specs: names }] of Object.entries(narrowFlagSets)) {
  for (const name of names) generate(name, set, flags);
}

const tsconfig = {
  compilerOptions: {
    strict: true,
    noEmit: true,
    target: "es2022",
    module: "esnext",
    moduleResolution: "bundler",
    skipLibCheck: true,
    noUncheckedIndexedAccess: true,
    exactOptionalPropertyTypes: false,
  },
  include: ["generated/*.ts", "fetch.test.ts"],
};
writeFileSync(join(here, "tsconfig.json"), `${JSON.stringify(tsconfig, null, 2)}\n`);

let failed = false;
for (const ts of ["ts6", "ts7"]) {
  try {
    execFileSync(join(here, "node_modules", ts, "bin/tsc"), ["-p", here], { stdio: "inherit" });
    console.log(`${ts}: ok`);
  } catch {
    console.log(`${ts}: type errors`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
