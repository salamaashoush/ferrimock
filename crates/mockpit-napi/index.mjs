import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const nativeModule = require("./index.js");
export const {
  http,
  graphql,
  MockResponse,
  MockpitServer,
  JsHandler,
  services,
} = nativeModule;
export default nativeModule;
