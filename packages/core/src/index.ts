// Core interceptor
export { MockpitInterceptor } from "./interceptor.js";
export type { ApplyOptions, UnhandledRequestStrategy } from "./interceptor.js";

// Lifecycle events
export { LifecycleEvents } from "./events.js";
export type { LifecycleEventMap } from "./events.js";

// MSW-compatible utilities
export { delay, passthrough, bypass } from "./msw-compat.js";

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

// GraphQL link (URL-scoped GraphQL handlers) — JS wrapper
export { graphqlLink } from "./graphql-link.js";

export type {
  JsHandler,
  MockpitRequest,
  JsHandlerResponse,
  JsResponseInit,
} from "@mockpit/node";
