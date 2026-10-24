// Serves this directory the way the remote-refs test does, for regenerating expected.ts:
//   node serve.mjs & npx --prefix ../../tools/reference openapi-typescript http://127.0.0.1:8471/api/openapi.yaml > expected.ts
import fs from "node:fs";
import http from "node:http";

const root = new URL("./", import.meta.url);
http
  .createServer((req, res) => {
    if (req.url === "/api/moved/owner.yaml") {
      res.writeHead(302, { location: "/api/schemas/owner.yaml" }).end();
      return;
    }
    const file = new URL(`.${req.url}`, root);
    if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end("not found");
      return;
    }
    const type = req.url.endsWith(".yaml") ? "text/plain; charset=utf-8" : "application/json";
    res.writeHead(200, { "content-type": type }).end(fs.readFileSync(file));
  })
  .listen(8471, "127.0.0.1");
