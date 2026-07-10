import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const nativeModule = require("./index.js");
export const {
  http,
  graphql,
  fake,
  ws,
  sse,
  HttpResponse,
  FerrimockServer,
  RequestHandler,
  RequestInfo,
  GraphQLRequestInfo,
  SseClientHandle,
  WebSocketClientHandle,
  WebSocketServerHandle,
  services,
  parseConfigFile,
  parseConfigString,
  discoverConfigFile,
} = nativeModule;
export default nativeModule;
