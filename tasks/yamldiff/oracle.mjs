// js-yaml oracle for the fuzzer: reads `u32le length + bytes` frames on stdin and answers each
// with one line from `expected()` (`OK <canonical>`, `ERR …` or `UNREPRESENTABLE …`).

import { expected } from "./jsyaml.mjs";

let buf = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  while (buf.length >= 4) {
    const n = buf.readUInt32LE(0);
    if (buf.length < 4 + n) break;
    const text = buf.subarray(4, 4 + n).toString("utf8");
    buf = buf.subarray(4 + n);
    process.stdout.write(expected(text).replace(/[\r\n]/g, " ") + "\n");
  }
});
