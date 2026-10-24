import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { execFile } from "node:child_process";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Readable } from "node:stream";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import ts from "typescript";
import openapiTS, { astToString, COMMENT_HEADER, generate, generateFile } from "./index.js";

const f = ts.factory;
const hooksDir = new URL("../../fixtures/hooks/", import.meta.url);
const yaml = `openapi: 3.0.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Pet: { type: string, enum: [cat, dog] }
    Toy: { type: object, properties: { n: { type: integer, default: 1 } } }
`;
const empty = { openapi: "3.1.0", info: { title: "t", version: "1" } };
const toTS = async (source, options) => astToString(await openapiTS(source, options));
const dir = mkdtempSync(join(tmpdir(), "spectype-test-"));
const write = (name, text) => {
  writeFileSync(join(dir, name), text);
  return join(dir, name);
};
const petFile = write("pet.yaml", "type: object\nproperties: { name: { type: string } }\n");
const rootFile = write(
  "root.yaml",
  `openapi: 3.1.0\ninfo: { title: t, version: "1" }\ncomponents:\n  schemas:\n    Pet: { $ref: "./pet.yaml" }\n`,
);

test("openapiTS output matches generate() minus the header", async () => {
  assert.equal(COMMENT_HEADER + (await toTS(yaml)), generate(yaml));
});

test("openapiTS accepts every source kind", async () => {
  const want = await toTS(yaml);
  assert.equal(await toTS(JSON.stringify(generateInput())), await toTS(generateInput()));
  assert.equal(await toTS(Buffer.from(yaml)), want);
  assert.equal(await toTS(Readable.from([yaml.slice(0, 20), yaml.slice(20)])), want);
  const file = write("spec.yaml", yaml);
  assert.equal(await toTS(pathToFileURL(file)), want);
  assert.equal(await toTS(pathToFileURL(file).href), want);
  await assert.rejects(openapiTS(""), { message: "Empty schema. Please specify a URL, file path, or Redocly Config" });
  await assert.rejects(openapiTS([1]), { message: "Expected string, object, or Buffer. Got Array" });
});

function generateInput() {
  return { ...empty, components: { schemas: { A: { type: "string" } } } };
}

test("openapiTS resolves file refs against the source or cwd", async () => {
  const out = await toTS(pathToFileURL(rootFile));
  assert.match(out, /Pet: components\["schemas"\]\["pet"\];\n {8}pet: \{\n {12}name\?: string;/);
  const fromString = readFileSync(rootFile, "utf8").replace("./pet.yaml", `./${dir.split("/").at(-1)}/pet.yaml`);
  assert.equal(await toTS(fromString, { cwd: dir }), out);
});

test("openapiTS fetches remote documents and their remote refs", async () => {
  const docs = {
    "/root.yaml": `openapi: 3.1.0\ninfo: { title: t, version: "1" }\ncomponents:\n  schemas:\n    Pet: { $ref: "./pet.yaml" }\n`,
    "/pet.yaml": `type: object\nproperties: { toy: { $ref: "toy.json" } }\n`,
    "/toy.json": `{ "type": "string" }`,
  };
  const server = createServer((req, res) => {
    const body = docs[req.url];
    res.writeHead(body ? 200 : 404, { "content-type": "text/plain" }).end(body ?? "");
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    assert.match(await toTS(new URL(`${base}/root.yaml`)), /toy: string;\n {8}pet: \{\n {12}toy\?: components/);
    assert.equal(await toTS(`${base}/root.yaml`, { transform: () => undefined }), await toTS(`${base}/root.yaml`));
    await assert.rejects(openapiTS(new URL(`${base}/missing.yaml`)), {
      message: `Failed to load ${base}/missing.yaml: 404 Not Found`,
    });
  } finally {
    server.close();
  }
});

test("remote requests send redocly resolve.http.headers, in the Node API and the CLI", async () => {
  const docs = {
    "/root.yaml": `openapi: 3.1.0\ninfo: { title: t, version: "1" }\ncomponents:\n  schemas:\n    Pet: { $ref: "./pet.yaml" }\n`,
    "/pet.yaml": `type: string\n`,
  };
  const seen = [];
  const server = createServer((req, res) => {
    seen.push(`${req.url} ${req.headers.authorization ?? "-"} ${req.headers["x-pet"] ?? "-"}`);
    res.end(docs[req.url] ?? "");
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const host = `127.0.0.1:${server.address().port}`;
  process.env.SPECTYPE_TEST_TOKEN = "env";
  const headers = [
    { matches: `${host}/**`, name: "Authorization", value: "Bearer a" },
    { matches: `http://${host}/pet.*`, name: "X-Pet", envVariable: "SPECTYPE_TEST_TOKEN" },
    { matches: "other.example.com/**", name: "Authorization", value: "Bearer b" },
  ];
  const want = ["/pet.yaml Bearer a env", "/root.yaml Bearer a -"];
  const cwd = mkdtempSync(join(tmpdir(), "spectype-headers-"));
  writeFileSync(join(cwd, "redocly.yaml"), JSON.stringify({ resolve: { http: { headers } } }));
  try {
    await toTS(`http://${host}/root.yaml`, { redocly: { resolve: { http: { headers } } } });
    assert.deepEqual(seen.splice(0).sort(), want);
    const bin = new URL("bin/spectype.js", import.meta.url).pathname;
    await promisify(execFile)(process.execPath, [bin, `http://${host}/root.yaml`], { cwd });
    assert.deepEqual(seen.splice(0).sort(), want);
  } finally {
    server.close();
  }
});

test("hooks see ctx.discriminators as openapi-typescript builds them", async () => {
  const doc = {
    ...empty,
    components: {
      schemas: {
        Pet: {
          oneOf: [{ $ref: "#/components/schemas/Cat" }, { $ref: "#/components/schemas/Dog" }],
          discriminator: { propertyName: "kind", mapping: { cat: "#/components/schemas/Cat" } },
        },
        Cat: { type: "object", properties: { kind: { type: "string" } } },
        Dog: { allOf: [{ $ref: "#/components/schemas/Pet" }, { type: "object" }] },
      },
    },
  };
  let seen;
  await openapiTS(doc, { transform: (_schema, options) => void (seen ??= options.ctx.discriminators) });
  const discriminator = { propertyName: "kind", mapping: { cat: "#/components/schemas/Cat" } };
  assert.deepEqual(seen, {
    objects: { "#/components/schemas/Pet": discriminator, "#/components/schemas/Dog": discriminator },
    refsHandled: ["#/components/schemas/Cat", "#/components/schemas/Dog"],
  });
});

test("hooks match openapi-typescript (fixtures/hooks/upstream.mjs)", async () => {
  const doc = JSON.parse(readFileSync(new URL("doc.json", hooksDir), "utf8"));
  const log = [];
  const json = () => {
    const a = f.createPropertySignature(undefined, "a", undefined, f.createKeywordTypeNode(ts.SyntaxKind.StringKeyword));
    ts.addSyntheticLeadingComment(a, ts.SyntaxKind.MultiLineCommentTrivia, "*\n\n * two\n ", true);
    const b = f.createPropertySignature(
      undefined,
      "b",
      f.createToken(ts.SyntaxKind.QuestionToken),
      f.createKeywordTypeNode(ts.SyntaxKind.NumberKeyword),
    );
    return f.createTypeLiteralNode([a, b]);
  };
  const out = await toTS(doc, {
    transform(schema, options) {
      log.push(`transform ${options.path} ${options.schema ? "schema" : "-"}`);
      if (schema.format === "date-time") return f.createTypeReferenceNode("Date");
      if (schema.format === "date") {
        return {
          schema: f.createUnionTypeNode([f.createTypeReferenceNode("Date"), f.createLiteralTypeNode(f.createNull())]),
          questionToken: true,
        };
      }
      if (schema.format === "json") return json();
    },
    postTransform(type, options) {
      if (options.path === "#/components/schemas/Wrapped") {
        return f.createUnionTypeNode([type, f.createTypeReferenceNode("Wrapped")]);
      }
    },
    transformProperty(property, _schema, options) {
      if (property.name.text === "id") {
        ts.addSyntheticLeadingComment(property, ts.SyntaxKind.SingleLineCommentTrivia, ` ${options.path}`, true);
        return property;
      }
    },
  });
  assert.equal(out, readFileSync(new URL("expected.ts", hooksDir), "utf8"));
  assert.equal(`${log.join("\n")}\n`, readFileSync(new URL("transform-calls.txt", hooksDir), "utf8"));
});

test("hooks get shared schema objects, the component and ctx", async () => {
  const doc = {
    ...empty,
    components: {
      schemas: { Pet: { type: "object", properties: { owner: { $ref: "#/components/schemas/Owner" } } }, Owner: {} },
    },
  };
  const seen = new Set();
  let resolved;
  await openapiTS(doc, {
    exportType: true,
    transform(schema, options) {
      seen.add(schema);
      assert.equal(options.ctx.exportType, true);
      assert.equal(options.ctx.defaultNonNullable, true);
      if (schema.type === "object") assert.equal(schema, options.schema);
      resolved ??= options.ctx.resolve("#/components/schemas/Owner");
    },
  });
  assert.deepEqual(resolved, {});
  assert.ok([...seen].some((s) => s.properties?.owner?.$ref === "#/components/schemas/Owner"));
});

test("postTransform sees a parsed type and can keep it", async () => {
  const kinds = new Set();
  const doc = { ...empty, components: { schemas: { S: { type: "string", enum: ["a", "b"] }, N: { type: "number" } } } };
  const out = await toTS(doc, {
    postTransform(type) {
      kinds.add(ts.SyntaxKind[type.kind]);
      return type;
    },
  });
  assert.equal(out, await toTS(doc));
  assert.deepEqual([...kinds].sort(), ["NumberKeyword", "UnionType"]);
});

test("hook errors propagate as thrown", async () => {
  const boom = new TypeError("boom");
  await assert.rejects(
    openapiTS(generateInput(), {
      transformProperty() {
        throw boom;
      },
      transform() {
        throw boom;
      },
    }),
    (e) => e === boom,
  );
});

test("inject is reprinted by the TypeScript printer", async () => {
  const out = await toTS(empty, { inject: "type Foo = string\n// gone\ntype Bar = {a:number}" });
  assert.ok(out.startsWith("type Foo = string;\ntype Bar = {\n    a: number;\n};\nexport type paths"));
});

test("astToString mixes generated output with TypeScript nodes", async () => {
  const extra = f.createTypeAliasDeclaration(undefined, "Extra", undefined, f.createKeywordTypeNode(ts.SyntaxKind.StringKeyword));
  const ast = await openapiTS(empty);
  assert.equal(astToString([...ast, extra]), `${astToString(ast)}type Extra = string;\n`);
});

test("errors carry openapi-typescript's messages and logs", async (t) => {
  const errors = t.mock.method(console, "error", () => {});
  await assert.rejects(openapiTS({ swagger: "2.0" }), { message: "Unsupported Swagger version: 2.x. Use OpenAPI 3.x instead." });
  const unresolved = {
    ...empty,
    components: {
      schemas: { Pet: { properties: { a: { $ref: "#/components/schemas/X" }, b: { $ref: "#/components/schemas/Y" } } } },
    },
  };
  await assert.rejects(openapiTS(unresolved), { message: "Can't resolve $ref at #/components/schemas/Pet/properties/b" });
  assert.equal(errors.mock.callCount(), 2);
  const op = { get: { operationId: "same", responses: {} } };
  const duplicate = { ...empty, paths: { "/a": op, "/b": op } };
  await assert.rejects(openapiTS(duplicate), /Every operation must have a unique `operationId`/);
  assert.match(generate(duplicate), /export interface paths/);
  assert.match(await toTS(duplicate, { redocly: { styleguide: { rules: { oas3_1: {} } } } }), /export interface paths/);
});

test("generate() maps options and fixes", () => {
  const out = generate(yaml, { exportType: true, enumValues: true, rootTypes: true, rootTypesNoSchemaPrefix: true });
  assert.match(out, /export type components = \{/);
  assert.match(out, /export const petValues/);
  assert.match(out, /export type Pet = components\['schemas'\]\['Pet'\];/);
  assert.match(generate(yaml), /n: number;/);
  assert.match(generate(yaml, { defaultNonNullable: false, immutable: undefined }), /n\?: number;/);
  assert.match(generate(yaml, { enumValues: true, fix: "all" }), /petValues: readonly \("cat" \| "dog"\)\[\]/);
  assert.match(generate(yaml, { enumValues: true, fix: ["enum-values-type"] }), /readonly \("cat"/);
  assert.throws(() => generate(yaml, { fix: ["nope"] }), /unknown fix "nope"/);
});

test("generateFile() resolves refs against the file", () => {
  assert.match(generateFile(rootFile), /name\?: string;/);
  assert.throws(() => generateFile(`${rootFile}.missing`), /^Error: Could not read/);
});

test("generate() throws the generator's error message", () => {
  assert.throws(() => generate("{}"), { message: "Unsupported schema format, expected `openapi: 3.x`" });
  assert.throws(() => generate(":: {"), /^Error: invalid YAML/);
});

test("CommonJS entry exposes the same API", async () => {
  const cjs = createRequire(import.meta.url)("./index.cjs");
  assert.equal(cjs.generate(yaml), generate(yaml));
  assert.equal(cjs.astToString(await cjs.default(yaml)), await toTS(yaml));
});
