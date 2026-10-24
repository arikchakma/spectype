"use strict";

// Loading the WebAssembly fallback makes Node print a WASI ExperimentalWarning; openapi-typescript prints nothing.
const { emitWarning } = process;
process.emitWarning = function (warning, ...rest) {
  if (!String(warning).startsWith("WASI is an experimental")) return emitWarning.call(this, warning, ...rest);
};
try {
  module.exports = require("./binding.cjs");
} finally {
  process.emitWarning = emitWarning;
}
