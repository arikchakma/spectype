import { copyFileSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [version, addons] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version ?? "") || !addons) {
  console.error("usage: node npm/scripts/stage.mjs <version> <dir with spectype.<platform>.node and .wasm>");
  process.exit(1);
}

const npm = join(import.meta.dirname, "..");
const platforms = readdirSync(join(npm, "platforms")).sort();
const writeVersion = (file, edit) => {
  const pkg = JSON.parse(readFileSync(file, "utf8"));
  pkg.version = version;
  edit?.(pkg);
  writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
};

for (const platform of platforms) {
  const addon = `spectype.${platform}.${platform.startsWith("wasm32") ? "wasm" : "node"}`;
  copyFileSync(join(addons, addon), join(npm, "platforms", platform, addon));
  writeVersion(join(npm, "platforms", platform, "package.json"));
}
writeVersion(join(npm, "spectype", "package.json"), (pkg) => {
  pkg.optionalDependencies = Object.fromEntries(platforms.map((p) => [`@spectype/binding-${p}`, version]));
});
console.log(`staged ${platforms.length} platform packages at ${version}`);
