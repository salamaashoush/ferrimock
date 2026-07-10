// Standalone Node (not bun) verification that http.request is intercepted.
// Run: node packages/core/verify-node-http.mjs
import http from "node:http";
import { FerrimockInterceptor } from "ferrimock";
import { http as mock, HttpResponse } from "@ferrimock/node";

function nodeRequest(url, options = {}, body) {
  return new Promise((resolve, reject) => {
    const req = http.request(url, options, (res) => {
      let data = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (data += c));
      res.on("end", () => resolve({ status: res.statusCode, body: data }));
    });
    req.on("error", reject);
    if (body) req.write(body);
    req.end();
  });
}

const interceptor = new FerrimockInterceptor();
interceptor.useHandlers([
  mock.get("/api/http", async () => HttpResponse.json({ ok: true, via: "node-http" })),
  mock.post("/echo", async ({ request }) => HttpResponse.json({ received: (await request.json()).name ?? null })),
]);
interceptor.apply();

let failures = 0;
const assert = (cond, msg) => {
  if (!cond) { console.error("FAIL:", msg); failures++; }
  else console.log("ok:", msg);
};

try {
  const get = await nodeRequest("http://example.test/api/http");
  assert(get.status === 200, `GET status 200 (got ${get.status})`);
  assert(JSON.parse(get.body).via === "node-http", "GET body mocked");

  const post = await nodeRequest(
    "http://example.test/echo",
    { method: "POST", headers: { "content-type": "application/json" } },
    JSON.stringify({ name: "ada" })
  );
  assert(JSON.parse(post.body).received === "ada", "POST body forwarded + mocked");
} catch (e) {
  console.error("ERROR:", e);
  failures++;
} finally {
  interceptor.dispose();
}

console.log(failures === 0 ? "\nALL PASS" : `\n${failures} FAILED`);
process.exit(failures === 0 ? 0 : 1);
