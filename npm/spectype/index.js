import { createRequire } from "node:module";

const cjs = createRequire(import.meta.url)("./index.cjs");

export const { openapiTS, astToString, COMMENT_HEADER, generate, generateFile } = cjs;
export default cjs.default;
