#!/usr/bin/env node
import { createRequire } from "node:module";
import { MessageChannel, Worker, receiveMessageOnPort } from "node:worker_threads";

const { cli } = createRequire(import.meta.url)("../native.cjs");

const FETCH = `
const { workerData: { url, headers, done, port } } = require("node:worker_threads");
fetch(url, { headers: JSON.parse(headers) })
  .then(async (r) =>
    r.ok
      ? { text: await r.text(), mimeType: r.headers.get("content-type") ?? undefined }
      : { error: \`Failed to load \${url}: \${r.status} \${r.statusText}\` },
  )
  .catch((e) => ({ error: \`Failed to load \${url}: \${e.message}\` }))
  .then((message) => {
    port.postMessage(message);
    Atomics.store(done, 0, 1);
    Atomics.notify(done, 0);
  });
`;

/** The WebAssembly CLI has no network: it fetches through a worker while this thread waits. */
function fetchSync(url, headers) {
  const done = new Int32Array(new SharedArrayBuffer(4));
  const { port1, port2 } = new MessageChannel();
  const worker = new Worker(FETCH, { eval: true, workerData: { url, headers, done, port: port2 }, transferList: [port2] });
  Atomics.wait(done, 0, 0);
  worker.terminate();
  const { error, ...document } = receiveMessageOnPort(port1).message;
  if (error) throw new Error(error);
  return document;
}

for (const stream of [process.stdin, process.stdout, process.stderr]) stream._handle?.setBlocking?.(true);
process.exitCode = cli(process.argv.slice(2), process.cwd(), { fetch: fetchSync }) ? 0 : 1;
