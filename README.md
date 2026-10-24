# spectype

A drop-in replacement for [`openapi-typescript`](https://github.com/openapi-ts/openapi-typescript)
7.13.0. It is about 18× faster and works with TypeScript 7, which openapi-typescript can't load
(issue #2841). Your generated file does not change, so `openapi-fetch` and your existing code keep
working.

## Install

```sh
npm install --save-dev spectype
```

npm installs a native addon on macOS, Linux and Windows, and a WebAssembly build everywhere
else.

## CLI

Replace `openapi-typescript` with `spectype`. Every flag and `redocly.yaml` work the same.

```sh
spectype openapi.yaml -o src/schema.ts
spectype https://example.com/openapi.json -o schema.ts
cat openapi.json | spectype > schema.ts
spectype openapi.yaml -o src/schema.ts --check   # exit 1 if the file is out of date
```

spectype adds two flags:

| Flag              | Effect                                                                           |
| ----------------- | -------------------------------------------------------------------------------- |
| `--fix=all`       | Fix openapi-typescript bugs instead of copying them (or name fixes: `--fix=a,b`) |
| `--inject <file>` | Insert the TypeScript in `<file>` before the generated types                     |

## Node API

Replace the import. Options, inputs and errors are the same.

```js
import openapiTS, { astToString } from "spectype";

const ast = await openapiTS(new URL("./openapi.yaml", import.meta.url));
const contents = astToString(ast);
```

There is also a synchronous API. It returns the finished file, like the CLI, but skips lint rules
and remote `$ref`s.

```js
import { generate, generateFile } from "spectype";

const code = generate(specText, { enumValues: true });
const fromFile = generateFile("openapi.yaml", { fix: "all" });
```

Differences from openapi-typescript:

- `transform`, `postTransform` and `transformProperty` need `typescript` 5 or 6 installed.
- `openapiTS` returns opaque nodes. Print them with `astToString` and don't inspect them.
- Internal helpers (`transform*`, `tsUnion`, `NULL`, …) are not exported.

## Benchmark

CLI wall time (median of 5 cold runs) and peak memory, on an Apple Silicon Mac with Node 24.

| Spec                                                                 | Size               | openapi-typescript | spectype        | Speedup |
| -------------------------------------------------------------------- | ------------------ | ------------------ | --------------- | ------- |
| [Stripe](https://github.com/stripe/openapi)                          | 5.2 MB             | 778 ms · 385 MB    | 43 ms · 56 MB   | 18×     |
| [GitHub](https://github.com/github/rest-api-description)             | 8.1 MB             | 1.24 s · 462 MB    | 72 ms · 80 MB   | 17×     |
| [Vercel](https://openapi.vercel.sh/)                                 | 11.3 MB            | 1.21 s · 482 MB    | 56 ms · 74 MB   | 22×     |
| [Cloudflare](https://github.com/cloudflare/api-schemas)              | 26.4 MB            | 3.13 s · 989 MB    | 173 ms · 214 MB | 18×     |
| [DigitalOcean](https://github.com/digitalocean/openapi) (multi-file) | 1.7 MB, 2034 files | 585 ms · 346 MB    | 72 ms · 25 MB   | 8×      |

Both tools produce the same output. Results for other generators and the Node API are in
[tasks/benchmark/results.md](tasks/benchmark/results.md). Run them with `just bench`.

## Compatibility

828 golden outputs are compared byte for byte against real openapi-typescript. They cover the
GitHub, Stripe, DigitalOcean and Octokit specs, upstream's fixtures and tests, and about 400
edge cases, with many flag combinations. Every known difference and every copied bug is in
[COMPAT.md](COMPAT.md).

## Development

```sh
just ready                      # format, lint, test, docs, golden suite; fails if anything changed
cargo test --workspace          # unit tests and golden suite
just bench                      # benchmark against other generators
```

Goldens come from `openapi-typescript@7.13.0` in `tasks/reference`. Don't edit them by hand.

## License

MIT
