import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "../..");
const WORK = join(HERE, ".work");
const BIN = join(HERE, "node_modules/.bin");
const require = createRequire(import.meta.url);

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
}

function pkgVersion(name) {
  return JSON.parse(readFileSync(join(HERE, "node_modules", name, "package.json"), "utf8")).version;
}

function spectypeBin() {
  return process.env.SPECTYPE_BIN ?? join(ROOT, "target/release/spectype");
}

function kiotaBin() {
  if (process.env.KIOTA_BIN) return process.env.KIOTA_BIN;
  const local = join(WORK, "kiota/kiota");
  if (existsSync(local)) return local;
  try {
    return execFileSync("which", ["kiota"], { encoding: "utf8" }).trim() || null;
  } catch {
    return null;
  }
}

function binVersion(bin, args = ["--version"]) {
  try {
    return execFileSync(bin, args, { encoding: "utf8" }).match(/\d+\.\d+\.\d+/)?.[0] ?? "?";
  } catch {
    return null;
  }
}

function tools() {
  const kiota = kiotaBin();
  return [
    {
      name: "spectype",
      version: () => binVersion(spectypeBin()),
      config: "`spectype <in> -o out.ts`",
      cmd: (i, o) => [spectypeBin(), [i, "-o", join(o, "schema.ts")]],
    },
    {
      name: "openapi-typescript",
      version: () => pkgVersion("openapi-typescript"),
      config: "`openapi-typescript <in> -o out.ts` (typescript@" + pkgVersion("typescript") + ")",
      cmd: (i, o) => [join(BIN, "openapi-typescript"), [i, "-o", join(o, "schema.ts")]],
    },
    {
      name: "@hey-api/openapi-ts",
      version: () => pkgVersion("@hey-api/openapi-ts"),
      config: "`openapi-ts -i <in> -o out -p @hey-api/typescript -s --no-log-file` (types plugin only, no SDK/client)",
      cmd: (i, o) => [join(BIN, "openapi-ts"), ["-i", i, "-o", o, "-p", "@hey-api/typescript", "-s", "--no-log-file"]],
    },
    {
      name: "swagger-typescript-api",
      version: () => pkgVersion("swagger-typescript-api"),
      config: "`swagger-typescript-api generate -p <in> -o out -n types.ts --no-client --route-types --silent`",
      cmd: (i, o) => [
        join(BIN, "swagger-typescript-api"),
        ["generate", "-p", i, "-o", o, "-n", "types.ts", "--no-client", "--route-types", "--silent"],
      ],
    },
    {
      name: "orval",
      version: () => pkgVersion("orval"),
      config: "`orval -c configs/orval.config.mjs` (client fetch, mode split, externalRefs allow *; no types-only mode, models land in api.schemas.ts but fetch functions are still generated)",
      cmd: (i, o) => [join(BIN, "orval"), ["-c", join(HERE, "configs/orval.config.mjs"), "--log-level", "error"], { BENCH_INPUT: i, BENCH_OUT: o }],
    },
    {
      name: "kubb",
      version: () => `${pkgVersion("@kubb/cli")} (plugin-ts ${pkgVersion("@kubb/plugin-ts")})`,
      config: "`kubb generate -c configs/kubb.config.mjs -s --reporter cli` with only `pluginTs({ output: { barrel: false } })`",
      cmd: (i, o) => [
        join(BIN, "kubb"),
        ["generate", "-c", join(HERE, "configs/kubb.config.mjs"), "-s", "--reporter", "cli"],
        { BENCH_INPUT: i, BENCH_OUT: o },
      ],
    },
    {
      name: "oazapfts",
      version: () => pkgVersion("oazapfts"),
      config: "`oazapfts <in> out.ts` (no types-only mode; emits fetch functions + types)",
      cmd: (i, o) => [join(BIN, "oazapfts"), [i, join(o, "api.ts")]],
    },
    {
      name: "typed-openapi",
      version: () => pkgVersion("typed-openapi"),
      config: "`typed-openapi <in> -o out.ts -r none` (pure types runtime; includes endpoint map + tiny client)",
      cmd: (i, o) => [join(BIN, "typed-openapi"), [i, "-o", join(o, "api.ts"), "-r", "none"]],
    },
    {
      name: "openapi-typescript-codegen",
      version: () => pkgVersion("openapi-typescript-codegen"),
      config: "`openapi -i <in> -o out --exportCore false --exportServices false --useUnionTypes` (models only)",
      cmd: (i, o) => [join(BIN, "openapi"), ["-i", i, "-o", o, "--exportCore", "false", "--exportServices", "false", "--useUnionTypes"]],
    },
    {
      name: "kiota (.NET native)",
      version: () => (kiota ? binVersion(kiota) : null),
      config: "`kiota generate -l typescript -d <in> -o out -n Api --ll Error --co` (no types-only mode; full client + models)",
      cmd: kiota && ((i, o) => [kiota, ["generate", "-l", "typescript", "-d", i, "-o", o, "-n", "Api", "--ll", "Error", "--co"]]),
    },
  ];
}

const REMOTE_INPUTS = [
  { name: "vercel", file: "vercel-api.json", url: "https://openapi.vercel.sh/" },
  {
    name: "cloudflare",
    file: "cloudflare-api.json",
    url: "https://raw.githubusercontent.com/cloudflare/api-schemas/5983a433f54f94d7a22479abd1478f518ec8b78f/openapi.json",
  },
];

async function downloadInputs() {
  mkdirSync(WORK, { recursive: true });
  for (const { file, url } of REMOTE_INPUTS) {
    const path = join(WORK, file);
    if (existsSync(path)) continue;
    process.stderr.write(`downloading ${url}\n`);
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: ${res.status} ${res.statusText}`);
    writeFileSync(path, await res.text());
  }
}

function inputs() {
  const ex = join(ROOT, "fixtures/ots/examples");
  const githubJson = join(WORK, "github-api.json");
  const githubYaml = join(ex, "github-api.yaml");
  if (!existsSync(githubJson) || statSync(githubJson).mtimeMs < statSync(githubYaml).mtimeMs) {
    const yaml = require("js-yaml");
    mkdirSync(WORK, { recursive: true });
    writeFileSync(githubJson, JSON.stringify(yaml.load(readFileSync(githubYaml, "utf8")), null, 2));
  }
  return [
    { name: "stripe", path: join(ex, "stripe-api.yaml") },
    { name: "github", path: githubYaml },
    { name: "github-json", path: githubJson },
    { name: "digital-ocean (multi-file)", path: join(ex, "digital-ocean-api/DigitalOcean-public.v2.yaml") },
    ...REMOTE_INPUTS.map(({ name, file }) => ({ name, path: join(WORK, file) })),
  ];
}

function outputBytes(p) {
  if (!existsSync(p)) return 0;
  const s = statSync(p);
  if (!s.isDirectory()) return s.size;
  return readdirSync(p).reduce((n, f) => n + outputBytes(join(p, f)), 0);
}

const ENV = {
  ...process.env,
  FORCE_COLOR: "0",
  NO_COLOR: "1",
  DO_NOT_TRACK: "1",
  KUBB_DISABLE_TELEMETRY: "1",
  KIOTA_TUTORIAL_ENABLED: "false",
  KIOTA_OFFLINE_ENABLED: "true",
  NO_UPDATE_NOTIFIER: "1",
};

function timed(bin, args, env, timeoutMs) {
  const timeFlag = platform() === "darwin" ? "-l" : "-v";
  return new Promise((done) => {
    const start = process.hrtime.bigint();
    const child = spawn("/usr/bin/time", [timeFlag, bin, ...args], {
      cwd: WORK,
      env: { ...ENV, ...env },
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    let stdout = "";
    child.stderr.on("data", (d) => (stderr += d));
    child.stdout.on("data", (d) => (stdout = (stdout + d).slice(-65536)));
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      try {
        process.kill(-child.pid, "SIGKILL");
      } catch {}
    }, timeoutMs);
    child.on("close", (code) => {
      clearTimeout(timer);
      const ms = Number(process.hrtime.bigint() - start) / 1e6;
      const mac = stderr.match(/(\d+)\s+maximum resident set size/);
      const linux = stderr.match(/Maximum resident set size \(kbytes\): (\d+)/);
      const rss = mac ? Number(mac[1]) : linux ? Number(linux[1]) * 1024 : null;
      done({ ms, rss, code, timedOut, stderr, stdout });
    });
  });
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

async function bench(name, runOnce, runs) {
  const samples = [];
  for (let r = 0; r < runs; r++) {
    const res = await runOnce();
    if (res.error) return { error: res.error };
    samples.push(res);
  }
  const ms = samples.map((s) => s.ms);
  return {
    median: median(ms),
    min: Math.min(...ms),
    rss: Math.max(...samples.map((s) => s.rss ?? 0)) || null,
    samples: ms,
    bytes: samples.at(-1).bytes,
  };
}

function lastLines(text, n = 3) {
  const lines = text.split("\n").filter((l) => l.trim() && !/^\s+\d+\s+[a-z]/.test(l) && !/^\s+[\d.]+ (real|user|sys)/.test(l));
  const err = lines.find((l) => /error|ERR_|fail/i.test(l) && !/^\s+at /.test(l));
  return (err ? [err.trim()] : lines.slice(-n)).join(" | ").slice(0, 300);
}

async function main() {
  await downloadInputs();
  const runs = Number(arg("runs", 5));
  const timeoutMs = Number(arg("timeout", 120)) * 1000;
  const only = arg("only")?.split(",");
  const onlyInputs = arg("inputs")?.split(",");
  const skipApi = process.argv.includes("--no-api");
  const all = tools().filter((t) => !only || only.some((o) => t.name.startsWith(o)));
  const results = { date: new Date().toISOString(), node: process.version, platform: `${platform()} ${process.arch}`, runs, timeoutMs, tools: {}, inputs: [], api: [] };

  for (const t of all) {
    const version = t.version();
    results.tools[t.name] = { version, config: t.config, available: Boolean(t.cmd && version) };
  }

  for (const input of inputs().filter((i) => !onlyInputs || onlyInputs.some((o) => i.name.startsWith(o)))) {
    const entry = { name: input.name, path: input.path.replace(ROOT + "/", ""), size: input.path.endsWith(".v2.yaml") ? null : statSync(input.path).size, rows: [] };
    results.inputs.push(entry);
    for (const t of all) {
      if (!results.tools[t.name].available) {
        entry.rows.push({ tool: t.name, error: "not installed" });
        continue;
      }
      const out = join(WORK, "out", t.name.replace(/[^\w.-]+/g, "_"));
      const [bin, args, env] = t.cmd(input.path, out);
      process.stderr.write(`${input.name} :: ${t.name} `);
      const res = await bench(
        t.name,
        async () => {
          rmSync(out, { recursive: true, force: true });
          mkdirSync(out, { recursive: true });
          const r = await timed(bin, args, env, timeoutMs);
          if (r.timedOut) return { error: `timeout (${timeoutMs / 1000}s)` };
          if (r.code !== 0) return { error: `exit ${r.code}: ${lastLines(r.stderr + "\n" + r.stdout)}` };
          const bytes = outputBytes(out);
          if (bytes < 1024) return { error: `empty output (${bytes} bytes)` };
          process.stderr.write(".");
          return { ...r, bytes };
        },
        runs,
      );
      process.stderr.write(res.error ? ` FAIL ${res.error}\n` : ` ${res.median.toFixed(0)}ms\n`);
      entry.rows.push({ tool: t.name, ...res });
    }
  }

  if (!skipApi) {
    for (const input of results.inputs) {
      const path = inputs().find((i) => i.name === input.name).path;
      const row = { name: input.name, rows: [] };
      for (const impl of ["spectype", "openapi-typescript"]) {
        process.stderr.write(`api ${input.name} :: ${impl} `);
        const r = await timed(process.execPath, [join(HERE, "run.mjs"), "--api", impl, path, String(runs)], {}, timeoutMs * (runs + 1));
        let res;
        try {
          res = r.code === 0 ? { ...JSON.parse(r.stderr.split("\n").find((l) => l.startsWith("{"))), rss: r.rss } : { error: r.timedOut ? "timeout" : `exit ${r.code}: ${lastLines(r.stderr)}` };
        } catch {
          res = { error: lastLines(r.stderr) };
        }
        process.stderr.write(res.error ? ` FAIL ${res.error}\n` : ` ${res.median.toFixed(0)}ms\n`);
        row.rows.push({ tool: impl, ...res });
      }
      results.api.push(row);
    }
  }

  writeFileSync(join(HERE, "results.json"), JSON.stringify(results, null, 2) + "\n");
  writeFileSync(join(HERE, "results.md"), markdown(results));
  process.stderr.write(`wrote ${join(HERE, "results.md")}\n`);
}

async function apiWorker(impl, file, runs) {
  const mod =
    impl === "spectype"
      ? await import(pathToFileURL(join(ROOT, "npm/spectype/index.js")).href)
      : await import("openapi-typescript");
  const { default: openapiTS, astToString } = mod;
  const url = pathToFileURL(file);
  const once = async () => {
    const start = performance.now();
    const text = astToString(await openapiTS(url, { silent: true }));
    if (!text.length) throw new Error("empty output");
    return { ms: performance.now() - start, bytes: Buffer.byteLength(text) };
  };
  const warm = await once();
  const samples = [];
  for (let i = 0; i < Number(runs); i++) samples.push(await once());
  const ms = samples.map((s) => s.ms);
  console.error(JSON.stringify({ first: warm.ms, median: median(ms), min: Math.min(...ms), samples: ms, bytes: warm.bytes }));
}

const fmtMs = (ms) => (ms >= 1000 ? `${(ms / 1000).toFixed(2)} s` : `${ms.toFixed(0)} ms`);
const fmtMb = (b) => (b ? `${(b / 1048576).toFixed(0)} MB` : "-");

function table(rows, base, extraCol) {
  const head = ["Tool", "Version", "Median", "Min", "Peak RSS", "vs spectype", ...(extraCol ? [extraCol.title] : [])];
  const lines = [`| ${head.join(" | ")} |`, `|${head.map(() => "---").join("|")}|`];
  const ok = rows.filter((r) => !r.error).sort((a, b) => a.median - b.median);
  for (const r of [...ok, ...rows.filter((r) => r.error)]) {
    const rel = r.error || !base ? "" : r === base ? "1.00x" : `${(r.median / base.median).toFixed(2)}x`;
    const cells = r.error
      ? [r.tool, r.version ?? "", "FAIL", "", "", r.error.replace(/\|/g, "\\|")]
      : [r.tool, r.version ?? "", fmtMs(r.median), fmtMs(r.min), fmtMb(r.rss), rel];
    if (extraCol) cells.push(r.error ? "" : extraCol.cell(r));
    lines.push(`| ${cells.join(" | ")} |`);
  }
  return lines.join("\n");
}

function markdown(res) {
  const v = (name) => res.tools[name]?.version;
  let md = `# OpenAPI → TypeScript benchmark\n\n${res.date} · node ${res.node} · ${res.platform} · ${res.runs} runs per cell (cold process each), timeout ${res.timeoutMs / 1000}s\n\n`;
  md += `Reproduce: \`cd tasks/benchmark && npm install && node run.mjs [--runs 5] [--timeout 120] [--only spectype,orval] [--inputs stripe] [--no-api]\`\n\n`;
  md += `Wall time is measured around a cold process start (includes Node/runtime boot). Peak RSS is the max across runs from \`/usr/bin/time\`.\n\n`;
  for (const input of res.inputs) {
    const rows = input.rows.map((r) => ({ ...r, version: v(r.tool) }));
    const base = rows.find((r) => r.tool === "spectype" && !r.error);
    md += `## ${input.name}\n\n\`${input.path}\`${input.size ? ` (${(input.size / 1048576).toFixed(1)} MB)` : ""}\n\n`;
    md += table(rows, base, { title: "Output", cell: (r) => `${(r.bytes / 1024).toFixed(0)} KB` }) + "\n\n";
  }
  if (res.api.length) {
    md += `## Node API, in-process (warm)\n\n\`astToString(await openapiTS(new URL(file)))\`, one warmup call then ${res.runs} timed calls in a fresh process per implementation. spectype = \`npm/spectype\` (native addon); openapi-typescript ${v("openapi-typescript")}. Peak RSS covers the whole worker process.\n\n`;
    for (const input of res.api) {
      const rows = input.rows.map((r) => ({ ...r, version: r.tool === "spectype" ? "npm/spectype (native addon)" : v(r.tool) }));
      const base = rows.find((r) => r.tool === "spectype" && !r.error);
      md += `### ${input.name}\n\n` + table(rows, base, { title: "First call", cell: (r) => fmtMs(r.first) }) + "\n\n";
    }
  }
  md += `## Tool configs\n\n| Tool | Version | Command |\n|---|---|---|\n`;
  for (const [name, t] of Object.entries(res.tools)) md += `| ${name} | ${t.version ?? "not installed"} | ${t.config} |\n`;
  md += `
## Notes

- Output sizes differ because the tools emit different things (spectype and openapi-typescript emit byte-identical \`paths\`/\`components\`/\`operations\` types; others emit per-model files, enums, or client code). Only orval, oazapfts and kiota lack a types-only mode, so they do extra work.
- vercel is \`https://openapi.vercel.sh/\` (unversioned; downloaded once into \`.work/\`, delete it to refresh) and cloudflare is \`openapi.json\` from [cloudflare/api-schemas](https://github.com/cloudflare/api-schemas) at 5983a43.
- A run counts as failed on non-zero exit, timeout, or output under 1 KB.
- kiota is a self-contained .NET binary, not on npm: \`gh release download v1.36.1 --repo microsoft/kiota -p osx-arm64.zip -D .work/kiota && unzip .work/kiota/osx-arm64.zip -d .work/kiota\` (or set \`KIOTA_BIN\`). Missing kiota is reported as "not installed".
- Skipped: openapi-generator-cli (needs Java; no JRE on this machine), openapi-zod-client (emits zod schemas, not types), autorest (heavy, .NET + extension download), vika-cli (Rust; \`generate\` needs an interactive module picker), @rtk-query/codegen-openapi (endpoint codegen, needs an RTK api slice).
- \`SPECTYPE_BIN\` overrides the CLI path (default \`target/release/spectype\`); the Node API worker imports \`npm/spectype/index.js\` (build its addon with \`npm run build --prefix npm\`).
`;
  return md;
}

if (process.argv.includes("--md-only")) writeFileSync(join(HERE, "results.md"), markdown(JSON.parse(readFileSync(join(HERE, "results.json"), "utf8"))));
else if (process.argv[2] === "--api") await apiWorker(...process.argv.slice(3));
else await main();
