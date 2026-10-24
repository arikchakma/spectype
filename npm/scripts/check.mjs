import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "../..");
const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const expected = cargo.match(/^\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m)?.[1];
if (!expected) throw new Error("no [workspace.package] version in Cargo.toml");

const read = (path) => JSON.parse(readFileSync(join(root, path), "utf8"));
const main = read("npm/spectype/package.json");
const platforms = readdirSync(join(root, "npm/platforms"));
const errors = [];
const triples = new Set(main.napi.targets);
if (triples.size !== platforms.length) errors.push(`napi.targets has ${triples.size} targets for ${platforms.length} platforms`);
const expect = (what, actual) => actual !== expected && errors.push(`${what} is ${actual}, expected ${expected}`);

expect("spectype", main.version);
for (const platform of platforms) {
  const name = `@spectype/binding-${platform}`;
  expect(name, read(`npm/platforms/${platform}/package.json`).version);
  expect(`spectype's optionalDependencies["${name}"]`, main.optionalDependencies[name]);
}
for (const name of Object.keys(main.optionalDependencies)) {
  if (!platforms.includes(name.replace("@spectype/binding-", ""))) errors.push(`${name} has no npm/platforms directory`);
}

if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log(`npm packages agree with Cargo.toml version ${expected}`);
