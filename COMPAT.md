# Compatibility with openapi-typescript 7.13.0

spectype prints the same file as `openapi-typescript@7.13.0` (on TypeScript 6), byte for byte,
before any formatter runs. This page lists every exception.

How it is checked:

- **Golden suite:** `cargo test -p spectype --test golden` compares 828 cases with what real
  openapi-typescript printed. Only the cases in `tests/known-differences.txt` may differ.
- **Differential fuzzer:** `tasks/difftest/run.mjs` feeds random documents to both tools. About
  40,000 runs so far.
- **Type check:** `tests/typecheck` compiles the GitHub, Stripe and Octokit output with
  `--fix=all` on tsc 6 and tsc 7.

## Differences

| Case | openapi-typescript | spectype |
| --- | --- | --- |
| A key named `hasOwnProperty`, `valueOf`, `toLocaleString` or another `Object.prototype` method | Crashes with `TypeError` | Generates types |
| A wrongly typed field, such as `parameters: "nope"` | Crashes with `TypeError` | Treats it as empty, or reports where it is |
| A pointer segment that is not valid percent-encoding (a property named `100%`) | Crashes with `URIError` | Keeps the segment as written |
| Schemas nested more than 100 levels deep | Error | Allowed up to 1000 levels |
| `inject` text | Reformatted by the TypeScript printer | Inserted as written, trimmed |
| Progress lines (`✨`, `🚀`) | On stdout | On stderr, naming `spectype` |
| A failing lint | Prints a stack trace | Prints only the ` ✘ ` lines |
| `apis` mode | Runs the APIs at the same time, in any order | Runs them in config order |
| `apis` mode with `--check` | Stops at the first up-to-date API | Checks every API |
| A `$ref` under a key named `toString` or `constructor`, whose target a discriminator patches | The inlined copy gets the patch too | The inlined copy does not. Only the fuzzer has hit this. |

## Lint (`redocly.yaml`)

**Without `redocly.yaml`**, no lint rule runs, as in upstream. Only the checks that decide whether
output can exist apply:

- `openapi` must be a `3.x` string. Swagger 2.0 is rejected.
- Every `$ref`, including `discriminator.mapping` values, must resolve.
- Files must parse, and duplicate keys are an error.

**With `redocly.yaml`**, `extends` and `rules` merge the way Redocly does. Problems print with
Redocly's messages, order and JSON Pointers, and any error stops generation. `struct` and every
rule in `minimal` and `recommended` are supported. Other rules (the rest of `all`, `rule/…`
assertions, plugins) are skipped with a warning.

Limits:

- `no-invalid-media-type-examples` skips `pattern`s that use lookaround or backreferences.
- A `$ref` to a missing file says `Can't resolve $ref`, without Redocly's `ENOENT` detail.
- `resolve.doNotResolveExamples` is not supported.
- Rules set on an `apis` entry are ignored, as in upstream.

## Upstream bugs (`--fix`)

spectype copies these bugs by default, so your file does not change. Pass `--fix=<name>,…` or
`--fix=all` (Node API: `fix: [...]` or `fix: "all"`) to get the corrected output.

| Fix | Bug | With the fix |
| --- | --- | --- |
| `enum-values-type` | `--enum-values` arrays have types that don't compile on GitHub, Stripe and Octokit, and two enums can get the same name | `readonly ("a" \| "b")[]`, and duplicate names get `_2`, `_3` |
| `root-types-names` | Names like `Foo` and `foo` collapse into one alias, invalid names print as is, and quotes aren't escaped | Every component gets a valid, unique alias |
| `enum-names` | `--enum` member names collide (`a.b` and `a:b` both become `a_b`), and `null` falls back to a union | Unique names, and `null` gives `MyEnum \| null` |
| `array-length` | `--array-length` with `minItems < maxItems` gives tuples of length 0 to max−min | Tuples of length min to max |
| `operation-id-refs` | An operationId with `%xx` is decoded in one place but not the other, so the reference breaks | The operationId is used as written in both |
| `default-non-nullable-scope` | A property with a `default` stays optional when its path merely contains `parameters` or `requestBody` | Only real parameters and request bodies are exempt |
| `const-items` | `items: { const: [1, 2] }` gives `[1, 2]` | `[1, 2][]` |
| `allof-empty-objects` | An `allOf` member with no properties, such as `{ type: object }` or `{ required: [id] }`, prints as `Record<string, never>`, so every sibling property becomes `never` | The member is dropped, and its `required` applies to the siblings |
| `discriminator-oneof-base` | A schema with `allOf: [$ref Base]`, where `Base` has a discriminator and a `oneOf`, becomes a new variant named after itself, and `Omit<Base, …>` collapses the union | It stays `Base`, or `Base & {…}`, unless `Base` lists it as a variant |
| `allof-required` | `required: [a]` next to `allOf: [$ref A]` is dropped when a discriminator mapping names `A` | `WithRequired<A, "a">` |
| `nested-discriminators` | A base with its own discriminator mapping that also extends another discriminator base loses its mapping, so variants listed after the base get their schema name as the value | Every mapped variant gets its mapped value |
| `discriminator-declared-values` | A `oneOf` variant with no `mapping` entry gets its schema name as the value, replacing the `enum` or `const` it declares (`commentType: "TextComment"` instead of `"text"`) | The declared values are kept |
| `anyof-properties` | `anyOf` next to `properties` or `allOf` prints a union with them (`{ name?: string } \| Email \| Phone`), so an object matching either side passes | `{ name?: string } & (Email \| Phone)`, as `oneOf` already prints |
| `defs-optional` | A schema's `$defs` prints as a required property, so no real value matches the type | `$defs?:`, and `$ref`s into it read `NonNullable<X["$defs"]>["name"]` |
| `prefix-items` | `prefixItems` always prints a closed tuple, so arrays with more items are rejected | `[number, string, ...unknown[]]`, or `...T[]` for `items: T`. `items: false`, `unevaluatedItems: false` and a `maxItems` within the prefix keep it closed |
| `read-write-primitives` | `--read-write-markers`: `Readable` and `Writable` map a branded primitive (`string & { __brand: "Id" }`) as an object, so the result is no longer a `string` | Primitives pass through unchanged |
| `bundled-ref-names` | An external `$ref` through a path with `/` (`other.yaml#/paths/~1a/…/application~1json/schema`) becomes a component named `application~1json-schema`, but the type refers to `application/json-schema`, which doesn't exist | The component is named `application/json-schema` |

`--path-params-as-types` has no fix. Overlapping paths (`/a/${string}` and `/a/${string}/b`) give
`TS2413` errors, and any type that compiles would change what `paths[url]` returns.

## Kept as is

These look odd but produce the same types, so spectype keeps them:

- `--enum` keeps or drops a trailing invalid character depending on the previous enum name.
- `nullable: true` on a union prints `(A | B) | null`.
- Integer-like keys such as `"200"` come first in ascending order.
- Numbers are doubles: `12345678901234567890` prints as `12345678901234567000`.
- Files are read with YAML rules (`1e400` is a string, `yes` is a string). Stdin that starts with
  `{` is read as JSON.
