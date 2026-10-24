
/**
 * Which binding artifact the generated loader actually loaded: `'native'` for
 * a native addon, otherwise the `platformArchABI` of the WASI flavor. Every
 * flavor napi-rs can build is listed, because `NAPI_RS_NATIVE_LIBRARY_PATH`
 * can point the loader at a WASI artifact this package does not build itself.
 */
export declare const __napiBindingTarget: 'native' | 'wasm32-wasi' | 'wasm32-wasip1'

/**
 * Runs the CLI in `cwd` with `args` (without the program name). Returns whether it succeeded.
 *
 * WebAssembly does not inherit the process's working directory and has no network, so JS
 * passes `cwd` and a `host` whose synchronous `fetch(url, headers)` (`headers` as JSON
 * `[name, value]` pairs) returns `{ text, mimeType }` or throws. The native addon fetches by itself.
 */
export declare function cli(args: Array<string>, cwd: string, host: object): boolean

/**
 * Generates TypeScript from `text`, a string or a `Buffer` of UTF-8 decoded as `buffer.toString()`.
 *
 * `format` is `json`, `yaml` or `auto` (JSON when the text starts with `{`). `location` is the
 * document's absolute path or URL (for text without one, the working directory plus `/`, as
 * openapi-typescript does); relative `$ref`s resolve against it. `options` is a JSON
 * object with openapi-typescript's option names plus `lint` (`{ oas3_0, oas3_1 }` rule maps) and
 * `fix` (`"all"` or fix names). `host` must have `load(url)` for `http(s)` documents, returning
 * `{ text, mimeType }` or `null`; it may have `document(json)` and the hooks `transform`,
 * `postTransform` and `transformProperty`, which are only called when present.
 *
 * Deep nesting must hit the parser's depth limit rather than the stack's: natively the
 * document is generated on a thread with a big stack and host calls come back to this thread;
 * in WebAssembly the stack size is set at link time.
 */
export declare function generate(text: string | Buffer, format: string, location: string, options: string, host: object): Generated

/** What [`generate`] returns: the generated file and its warnings, or why it failed. */
export interface Generated {
  /** The generated file, header included. */
  code?: string
  /** Warnings, in the order openapi-typescript prints them. */
  warnings: Array<string>
  /** Set when generation failed. */
  error?: GenerateError
}

/** Why [`generate`] failed. */
export interface GenerateError {
  /** The error message. */
  message: string
  /** Every unresolved `$ref`, for an unresolved-ref failure. */
  pointers?: Array<string>
  /** Every lint problem, for a lint failure. */
  problems?: Array<Problem>
}

/** One lint problem. */
export interface Problem {
  /** The problem as Redocly prints it. */
  message: string
  /** Whether it is an error rather than a warning. */
  error: boolean
}

/** The property `transformProperty` receives, before JS turns it into a TypeScript node. */
export interface PropertyInput {
  /** `identifier`, `string` or `numeric`. */
  nameKind: string
  /** The property name. */
  name: string
  /** Whether the property is `readonly`. */
  readonly: boolean
  /** Whether the property is optional. */
  optional: boolean
  /** The property's type source. */
  type: string
  /** The `JSDoc` comment to attach after the hook. */
  jsdoc?: string
}

/**
 * The headers Redocly sends with a request for `url`, as `[name, value]` pairs. `headers` is
 * `resolve.http.headers` as JSON; an entry's `envVariable` must already be replaced by its
 * `value`.
 */
export declare function requestHeaders(url: string, headers: string): Array<Array<string>>
