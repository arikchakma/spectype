# OpenAPI → TypeScript benchmark

2026-10-24T00:54:05.212Z · node v24.22.1 · darwin arm64 · 5 runs per cell (cold process each), timeout 120s

Reproduce: `cd tasks/benchmark && npm install && node run.mjs [--runs 5] [--timeout 120] [--only spectype,orval] [--inputs stripe] [--no-api]`

Wall time is measured around a cold process start (includes Node/runtime boot). Peak RSS is the max across runs from `/usr/bin/time`.

## stripe

`fixtures/ots/examples/stripe-api.yaml` (5.2 MB)

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 43 ms | 43 ms | 56 MB | 1.00x | 3756 KB |
| orval | 8.44.0 | 720 ms | 718 ms | 374 MB | 16.75x | 5750 KB |
| openapi-typescript | 7.13.0 | 778 ms | 767 ms | 385 MB | 18.10x | 3756 KB |
| typed-openapi | 4.1.0 | 1.19 s | 1.19 s | 304 MB | 27.69x | 2280 KB |
| oazapfts | 7.5.0 | 1.21 s | 1.20 s | 349 MB | 28.05x | 2128 KB |
| openapi-typescript-codegen | 0.31.0 | 1.41 s | 1.39 s | 263 MB | 32.78x | 1441 KB |
| kubb | 5.12.4 (plugin-ts 5.0.14) | 2.03 s | 1.98 s | 550 MB | 47.22x | 4710 KB |
| @hey-api/openapi-ts | 0.99.0 | 2.99 s | 2.97 s | 708 MB | 69.65x | 2944 KB |
| swagger-typescript-api | 13.13.0 | 3.87 s | 3.83 s | 842 MB | 90.14x | 2836 KB |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## github

`fixtures/ots/examples/github-api.yaml` (8.1 MB)

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 72 ms | 71 ms | 80 MB | 1.00x | 5390 KB |
| typed-openapi | 4.1.0 | 789 ms | 773 ms | 313 MB | 10.91x | 1674 KB |
| orval | 8.44.0 | 1.08 s | 1.05 s | 448 MB | 14.91x | 5164 KB |
| openapi-typescript-codegen | 0.31.0 | 1.17 s | 1.06 s | 481 MB | 16.24x | 2075 KB |
| openapi-typescript | 7.13.0 | 1.24 s | 1.22 s | 462 MB | 17.16x | 5390 KB |
| @hey-api/openapi-ts | 0.99.0 | 1.47 s | 1.45 s | 588 MB | 20.32x | 3308 KB |
| kubb | 5.12.4 (plugin-ts 5.0.14) | 2.89 s | 2.80 s | 730 MB | 39.94x | 6891 KB |
| swagger-typescript-api | 13.13.0 | 6.90 s | 6.84 s | 1136 MB | 95.41x | 3739 KB |
| oazapfts | 7.5.0 | FAIL |  |  | exit 1: Error: Discriminators require references, not inline schemas |  |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## github-json

`tasks/benchmark/.work/github-api.json` (10.7 MB)

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 62 ms | 59 ms | 84 MB | 1.00x | 5390 KB |
| typed-openapi | 4.1.0 | 734 ms | 731 ms | 256 MB | 11.92x | 1674 KB |
| orval | 8.44.0 | 929 ms | 923 ms | 352 MB | 15.08x | 5164 KB |
| openapi-typescript-codegen | 0.31.0 | 1.07 s | 1.03 s | 205 MB | 17.45x | 2075 KB |
| openapi-typescript | 7.13.0 | 1.22 s | 1.18 s | 492 MB | 19.83x | 5390 KB |
| @hey-api/openapi-ts | 0.99.0 | 1.37 s | 1.36 s | 621 MB | 22.26x | 3308 KB |
| kubb | 5.12.4 (plugin-ts 5.0.14) | 1.83 s | 1.81 s | 401 MB | 29.78x | 6891 KB |
| swagger-typescript-api | 13.13.0 | 5.74 s | 5.52 s | 820 MB | 93.12x | 3739 KB |
| oazapfts | 7.5.0 | FAIL |  |  | exit 1: Error: Discriminators require references, not inline schemas |  |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## digital-ocean (multi-file)

`fixtures/ots/examples/digital-ocean-api/DigitalOcean-public.v2.yaml`

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 72 ms | 67 ms | 25 MB | 1.00x | 1424 KB |
| openapi-typescript | 7.13.0 | 585 ms | 579 ms | 346 MB | 8.15x | 1424 KB |
| swagger-typescript-api | 13.13.0 | 747 ms | 733 ms | 400 MB | 10.41x | 151 KB |
| orval | 8.44.0 | 1.33 s | 1.32 s | 386 MB | 18.47x | 6295 KB |
| @hey-api/openapi-ts | 0.99.0 | FAIL |  |  | exit 1: Unexpected error: JobError |  |
| kubb | 5.12.4 (plugin-ts 5.0.14) | FAIL |  |  | exit 1: error KUBB_ADAPTER_FAILED: adapter "oas" crashed: Cannot use 'in' operator to search for 'const' in #/components/schemas/load_balancer_create/oneOf/0/allOf/0/properties |  |
| oazapfts | 7.5.0 | FAIL |  |  | exit 1: Error: Discriminators require references, not inline schemas |  |
| typed-openapi | 4.1.0 | FAIL |  |  | exit 1: throw new Error(`Unresolved ref "${name}" not found in "${path}"`); |  |
| openapi-typescript-codegen | 0.31.0 | FAIL |  |  | empty output (133 bytes) |  |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## vercel

`tasks/benchmark/.work/vercel-api.json` (11.3 MB)

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 56 ms | 54 ms | 74 MB | 1.00x | 5370 KB |
| openapi-typescript-codegen | 0.31.0 | 389 ms | 365 ms | 227 MB | 6.91x | 437 KB |
| typed-openapi | 4.1.0 | 430 ms | 411 ms | 230 MB | 7.62x | 2865 KB |
| oazapfts | 7.5.0 | 585 ms | 574 ms | 374 MB | 10.38x | 3104 KB |
| @hey-api/openapi-ts | 0.99.0 | 1.21 s | 1.19 s | 560 MB | 21.47x | 3566 KB |
| openapi-typescript | 7.13.0 | 1.21 s | 1.19 s | 482 MB | 21.53x | 5370 KB |
| kubb | 5.12.4 (plugin-ts 5.0.14) | 1.56 s | 1.54 s | 434 MB | 27.73x | 4958 KB |
| swagger-typescript-api | 13.13.0 | 4.85 s | 4.82 s | 774 MB | 86.04x | 3343 KB |
| orval | 8.44.0 | FAIL |  |  | exit 1: bench - 🛑 OpenAPI spec validation failed: |  |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## cloudflare

`tasks/benchmark/.work/cloudflare-api.json` (26.4 MB)

| Tool | Version | Median | Min | Peak RSS | vs spectype | Output |
|---|---|---|---|---|---|---|
| spectype | 0.1.0 | 173 ms | 163 ms | 214 MB | 1.00x | 15124 KB |
| orval | 8.44.0 | 2.39 s | 2.31 s | 740 MB | 13.81x | 16220 KB |
| typed-openapi | 4.1.0 | 2.95 s | 2.88 s | 449 MB | 17.04x | 7471 KB |
| openapi-typescript | 7.13.0 | 3.13 s | 3.09 s | 989 MB | 18.09x | 15124 KB |
| @hey-api/openapi-ts | 0.99.0 | 4.83 s | 4.83 s | 1484 MB | 27.96x | 10441 KB |
| kubb | 5.12.4 (plugin-ts 5.0.14) | 5.70 s | 4.44 s | 1172 MB | 32.96x | 17131 KB |
| openapi-typescript-codegen | 0.31.0 | 12.11 s | 11.25 s | 430 MB | 70.07x | 4569 KB |
| swagger-typescript-api | 13.13.0 | 23.17 s | 23.03 s | 1745 MB | 134.01x | 9357 KB |
| oazapfts | 7.5.0 | FAIL |  |  | exit 1: Error: Discriminators require references, not inline schemas |  |
| kiota (.NET native) |  | FAIL |  |  | not installed |  |

## Node API, in-process (warm)

`astToString(await openapiTS(new URL(file)))`, one warmup call then 5 timed calls in a fresh process per implementation. spectype = `npm/spectype` (native addon); openapi-typescript 7.13.0. Peak RSS covers the whole worker process.

### stripe

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 44 ms | 43 ms | 140 MB | 1.00x | 60 ms |
| openapi-typescript | 7.13.0 | 568 ms | 557 ms | 619 MB | 13.02x | 620 ms |

### github

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 75 ms | 72 ms | 204 MB | 1.00x | 81 ms |
| openapi-typescript | 7.13.0 | 971 ms | 946 ms | 743 MB | 12.91x | 1.05 s |

### github-json

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 60 ms | 60 ms | 209 MB | 1.00x | 69 ms |
| openapi-typescript | 7.13.0 | 958 ms | 953 ms | 761 MB | 16.00x | 1.03 s |

### digital-ocean (multi-file)

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 57 ms | 53 ms | 97 MB | 1.00x | 318 ms |
| openapi-typescript | 7.13.0 | 320 ms | 314 ms | 570 MB | 5.67x | 402 ms |

### vercel

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 55 ms | 54 ms | 206 MB | 1.00x | 63 ms |
| openapi-typescript | 7.13.0 | 970 ms | 931 ms | 680 MB | 17.73x | 1.05 s |

### cloudflare

| Tool | Version | Median | Min | Peak RSS | vs spectype | First call |
|---|---|---|---|---|---|---|
| spectype | npm/spectype (native addon) | 180 ms | 174 ms | 390 MB | 1.00x | 195 ms |
| openapi-typescript | 7.13.0 | 2.79 s | 2.77 s | 1138 MB | 15.49x | 2.87 s |

## Tool configs

| Tool | Version | Command |
|---|---|---|
| spectype | 0.1.0 | `spectype <in> -o out.ts` |
| openapi-typescript | 7.13.0 | `openapi-typescript <in> -o out.ts` (typescript@6.0.3) |
| @hey-api/openapi-ts | 0.99.0 | `openapi-ts -i <in> -o out -p @hey-api/typescript -s --no-log-file` (types plugin only, no SDK/client) |
| swagger-typescript-api | 13.13.0 | `swagger-typescript-api generate -p <in> -o out -n types.ts --no-client --route-types --silent` |
| orval | 8.44.0 | `orval -c configs/orval.config.mjs` (client fetch, mode split, externalRefs allow *; no types-only mode, models land in api.schemas.ts but fetch functions are still generated) |
| kubb | 5.12.4 (plugin-ts 5.0.14) | `kubb generate -c configs/kubb.config.mjs -s --reporter cli` with only `pluginTs({ output: { barrel: false } })` |
| oazapfts | 7.5.0 | `oazapfts <in> out.ts` (no types-only mode; emits fetch functions + types) |
| typed-openapi | 4.1.0 | `typed-openapi <in> -o out.ts -r none` (pure types runtime; includes endpoint map + tiny client) |
| openapi-typescript-codegen | 0.31.0 | `openapi -i <in> -o out --exportCore false --exportServices false --useUnionTypes` (models only) |
| kiota (.NET native) | not installed | `kiota generate -l typescript -d <in> -o out -n Api --ll Error --co` (no types-only mode; full client + models) |

## Notes

- Output sizes differ because the tools emit different things (spectype and openapi-typescript emit byte-identical `paths`/`components`/`operations` types; others emit per-model files, enums, or client code). Only orval, oazapfts and kiota lack a types-only mode, so they do extra work.
- vercel is `https://openapi.vercel.sh/` (unversioned; downloaded once into `.work/`, delete it to refresh) and cloudflare is `openapi.json` from [cloudflare/api-schemas](https://github.com/cloudflare/api-schemas) at 5983a43.
- A run counts as failed on non-zero exit, timeout, or output under 1 KB.
- kiota is a self-contained .NET binary, not on npm: `gh release download v1.36.1 --repo microsoft/kiota -p osx-arm64.zip -D .work/kiota && unzip .work/kiota/osx-arm64.zip -d .work/kiota` (or set `KIOTA_BIN`). Missing kiota is reported as "not installed".
- Skipped: openapi-generator-cli (needs Java; no JRE on this machine), openapi-zod-client (emits zod schemas, not types), autorest (heavy, .NET + extension download), vika-cli (Rust; `generate` needs an interactive module picker), @rtk-query/codegen-openapi (endpoint codegen, needs an RTK api slice).
- `SPECTYPE_BIN` overrides the CLI path (default `target/release/spectype`); the Node API worker imports `npm/spectype/index.js` (build its addon with `npm run build --prefix npm`).
