// Core interceptor
export { MockpitInterceptor } from "./interceptor.js";

// Config
export { defineConfig, loadConfig } from "./config.js";
export type { MockpitConfig } from "./config.js";

// Loader
export { loadMocksDir } from "./loader.js";

// Re-export everything from @mockpit/node
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
