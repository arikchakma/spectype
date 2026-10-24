import type { Readable } from "node:stream";
import type ts from "typescript";
import type { OpenAPI3, OpenAPITSOptions } from "./index.cjs";

export * from "./index.cjs";

/** {@inheritDoc openapiTS} */
export default function openapiTS(
  source: string | URL | OpenAPI3 | Buffer | Readable,
  options?: OpenAPITSOptions,
): Promise<ts.Node[]>;
