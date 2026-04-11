// Config
export { defineConfig } from "./config.js";
export type { MockpitConfig } from "./config.js";

// Loader
export { loadMocksDir } from "./loader.js";

// Re-export the handler API from @mockpit/node
export {
  http,
  graphql,
  fake,
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
