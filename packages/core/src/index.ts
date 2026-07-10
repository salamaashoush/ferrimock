// Core interceptor
export { FerrimockInterceptor } from "./interceptor.js";
export type {
  ApplyOptions,
  ListedHandler,
  UnhandledRequestStrategy,
} from "./interceptor.js";

// Standalone request resolution (MSW's getResponse/handleRequest)
export { getResponse, handleRequest, HttpMethods } from "./get-response.js";
export type { HandleRequestOptions } from "./get-response.js";

// MSW-compatible server (also exported from the ./node entry point)
export { setupServer } from "./node.js";
export type { SetupServerApi, ListenOptions } from "./node.js";

// Lifecycle events
export { LifecycleEvents } from "./events.js";
export type { LifecycleEventMap } from "./events.js";

// MSW-compatible utilities
export {
  delay,
  passthrough,
  bypass,
  cleanUrl,
  matchRequestUrl,
} from "./msw-compat.js";
export type { PathParams } from "./msw-compat.js";

// MSW-compatible Response subclass
export { HttpResponse } from "./http-response.js";
export type { StrictResponse } from "./http-response.js";

// Config
export { defineConfig, loadConfig } from "./config.js";
export type { FerrimockConfig } from "./config.js";

// Loader
export { loadMocksDir } from "./loader.js";

// Handler factories -- wrapped so CLI-style mock files (bare http.get
// calls) register under loadMocksDir, with MSW resolver semantics
// (Response returns, generators, fall-through); see registration.ts
export { http, graphql, collectHandlers } from "./registration.js";

// WebSocket interception (MSW's `ws` namespace)
export { ws, WebSocketHandler } from "./ws.js";
export type { WebSocketLink, WebSocketConnection } from "./ws.js";
export type {
  WebSocketData,
  WebSocketClientConnectionProtocol,
} from "@mswjs/interceptors/WebSocket";

// Server-Sent Events (MSW's `sse` namespace)
export { sse, ServerSentEventClient, ServerSentEventServer } from "./sse.js";
export type {
  ServerSentEventMessage,
  ServerSentEventResolverInfo,
} from "./sse.js";
export type { EventSourceLike } from "./event-source.js";

// Ferrimock-native surface (fake data, embedded server, services)
export { fake, FerrimockServer, services } from "@ferrimock/node";

export type {
  RequestHandler,
  RequestHandlerOptions,
  RequestInfo,
  GraphQLRequestInfo,
  HandlerResponse,
  HandlerResponseInit,
} from "@ferrimock/node";
