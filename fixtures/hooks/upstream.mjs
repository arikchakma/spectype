import fs from "node:fs";
const R = new URL("../../tools/reference/node_modules/", import.meta.url).href;
const { default: ts } = await import(R + "typescript/lib/typescript.js");
const { default: openapiTS, astToString } = await import(R + "openapi-typescript/dist/index.mjs");
const f = ts.factory;
const doc = JSON.parse(fs.readFileSync(new URL("./doc.json", import.meta.url), "utf8"));
const log = [];
const json = () => {
  const a = f.createPropertySignature(undefined, "a", undefined, f.createKeywordTypeNode(ts.SyntaxKind.StringKeyword));
  ts.addSyntheticLeadingComment(a, ts.SyntaxKind.MultiLineCommentTrivia, "*\n\n * two\n ", true);
  const b = f.createPropertySignature(undefined, "b", f.createToken(ts.SyntaxKind.QuestionToken), f.createKeywordTypeNode(ts.SyntaxKind.NumberKeyword));
  return f.createTypeLiteralNode([a, b]);
};
const out = astToString(await openapiTS(doc, {
  transform(schema, options) {
    log.push(`transform ${options.path} ${options.schema ? "schema" : "-"}`);
    if (schema.format === "date-time") return f.createTypeReferenceNode("Date");
    if (schema.format === "date") return { schema: f.createUnionTypeNode([f.createTypeReferenceNode("Date"), f.createLiteralTypeNode(f.createNull())]), questionToken: true };
    if (schema.format === "json") return json();
  },
  postTransform(type, options) {
    if (options.path === "#/components/schemas/Wrapped") {
      return f.createUnionTypeNode([type, f.createTypeReferenceNode("Wrapped")]);
    }
  },
  transformProperty(property, schema, options) {
    if (property.name.text === "id") {
      ts.addSyntheticLeadingComment(property, ts.SyntaxKind.SingleLineCommentTrivia, ` ${options.path}`, true);
      return property;
    }
  },
}));
fs.writeFileSync(new URL("./expected.ts", import.meta.url), out);
fs.writeFileSync(new URL("./transform-calls.txt", import.meta.url), log.join("\n") + "\n");
