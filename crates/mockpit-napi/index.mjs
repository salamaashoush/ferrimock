import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const nativeModule = require("./index.js");
export const {
  http,
  graphql,
  fake,
  MockResponse,
  MockpitServer,
  JsHandler,
  services,
  parseConfigFile,
  parseConfigString,
  discoverConfigFile,
} = nativeModule;
export default nativeModule;
