// Mockoon environments carry uuids and a migration marker, so the fixture is
// generated through Mockoon's own factories instead of hand-written JSON.
import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { BuildEnvironment, BuildHTTPRoute, BuildRouteResponse } = require("@mockoon/commons");

const here = path.dirname(fileURLToPath(import.meta.url));

const json = (body) => ({
  ...BuildRouteResponse(),
  body,
  statusCode: 200,
  headers: [{ key: "Content-Type", value: "application/json" }],
});

const route = (method, endpoint, responses) => ({
  ...BuildHTTPRoute(false),
  method,
  endpoint,
  responses,
});

const env = BuildEnvironment({ hasContentTypeHeader: true, hasRoutes: false, hasDefaultHeader: false });
env.name = "ferrimock-benchmark";
env.port = 4102;
env.hostname = "127.0.0.1";
env.routes = [
  route("get", "api/static", [
    json('{"id":1,"name":"John Smith","email":"john@example.com","active":true}'),
  ]),
  route("get", "api/users/:id", [
    json('{"id":"{{urlParam \'id\'}}","name":"John Smith","active":true}'),
  ]),
  route("get", "api/users/:id/profile", [
    json(
      '{"id":"{{urlParam \'id\'}}","name":"{{faker \'person.fullName\'}}","email":"{{faker \'internet.email\'}}","uuid":"{{faker \'string.uuid\'}}","city":"{{faker \'location.city\'}}"}',
    ),
  ]),
  route("get", "api/list", [
    json(
      '{"items":[{{#repeat 20 comma=true}}{"id":{{@index}},"name":"{{faker \'person.fullName\'}}","email":"{{faker \'internet.email\'}}"}{{/repeat}}]}',
    ),
  ]),
  route("post", "api/echo", [
    { ...json('{"received":{{{stringify (body)}}},"ok":true}'), statusCode: 201 },
  ]),
  // Mockoon picks the first response whose rules pass, so the admin variant is
  // listed first and the plain one acts as the fallback.
  route("get", "api/whoami", [
    {
      ...json('{"role":"admin"}'),
      rules: [
        {
          target: "header",
          modifier: "authorization",
          value: "^Bearer admin-.+",
          invert: false,
          operator: "regex",
        },
      ],
      rulesOperator: "AND",
    },
    json('{"role":"user"}'),
  ]),
];

const out = path.join(here, "environment.json");
writeFileSync(out, `${JSON.stringify(env, null, 2)}\n`);
console.log(`wrote ${out} (${env.routes.length} routes)`);
