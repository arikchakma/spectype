import type { Readable } from "node:stream";
import type ts from "typescript";
import type { Fix, OpenAPI3, OpenAPITSOptions } from "./types.cjs";

export * from "./types.cjs";

/** The header openapi-typescript's CLI prepends; {@link astToString} does not add it. */
export declare const COMMENT_HEADER: string;

/** Options for {@link generate} and {@link generateFile}: openapi-typescript's boolean options plus `fix`. */
export type SpectypeOptions = Pick<
  OpenAPITSOptions,
  | "additionalProperties"
  | "alphabetize"
  | "arrayLength"
  | "defaultNonNullable"
  | "emptyObjectsUnknown"
  | "enum"
  | "enumValues"
  | "conditionalEnums"
  | "dedupeEnums"
  | "excludeDeprecated"
  | "exportType"
  | "immutable"
  | "pathParamsAsTypes"
  | "propertiesRequiredByDefault"
  | "rootTypes"
  | "rootTypesNoSchemaPrefix"
  | "rootTypesKeepCasing"
  | "makePathsEnum"
  | "generatePathParams"
  | "readWriteMarkers"
  | "inject"
  | "fix"
>;

export type { Fix };

export interface AstToStringOptions {
  fileName?: string;
  sourceText?: string;
  formatOptions?: ts.PrinterOptions;
}

/**
 * Convert an OpenAPI 3.x document to TypeScript, like openapi-typescript's default export.
 *
 * @param source YAML or JSON text, an http(s) or file URL (string or `URL`), a `Buffer`, a
 *   `Readable`, or a parsed document.
 * @returns Statements to print with {@link astToString}. They are opaque: print them, optionally
 *   alongside your own TypeScript nodes, rather than inspecting them.
 */
declare function openapiTS(source: string | URL | OpenAPI3 | Buffer | Readable, options?: OpenAPITSOptions): Promise<ts.Node[]>;
export default openapiTS;
export { openapiTS };

/** Print what {@link openapiTS} returned, optionally mixed with TypeScript nodes. */
export declare function astToString(
  ast: ts.Node | ts.Node[] | ts.TypeElement | ts.TypeElement[],
  options?: AstToStringOptions,
): string;

/**
 * Generate TypeScript types (with {@link COMMENT_HEADER}) from an OpenAPI 3.x document, synchronously.
 * Like the CLI, it runs no lint rules. Remote `$ref`s are not fetched; use {@link openapiTS} for those.
 *
 * @param spec YAML or JSON document text, or an already-parsed document object.
 * @throws {Error} With the generator's message when the document cannot be converted.
 */
export declare function generate(spec: string | object, options?: SpectypeOptions): string;

/**
 * {@link generate} for a file on disk. Relative `$ref`s resolve against the file.
 *
 * @throws {Error} With the generator's message when the file cannot be read or converted.
 */
export declare function generateFile(path: string, options?: SpectypeOptions): string;
