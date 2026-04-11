// Re-export everything users need for mockpit.config.ts
export { defineConfig } from "./config.js";
export type { MockpitConfig } from "./config.js";

// Re-export the handler API from @mockpit/node
export {
  http,
  graphql,
  MockResponse,
  MockpitServer,
  services,
} from "@mockpit/node";

export type {
  JsHandler,
  JsRequestContext,
  JsHandlerResponse,
  JsResponseInit,
} from "@mockpit/node";
