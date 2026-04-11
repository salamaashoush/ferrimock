import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const testMatch = defineCommand({
  meta: { name: "test", description: "Test mock matching for a request", aliases: ["t"] },
  args: {
    path: { type: "positional" as const, required: true, description: "Request path" },
    method: { type: "string" as const, short: "m", default: "GET", description: "HTTP method" },
    query: { type: "string" as const, short: "q", description: "Query string" },
    header: { type: "string" as const, short: "H", action: "append" as const, description: 'Header (format: "Name: Value")' },
    body: { type: "string" as const, short: "b", description: "Request body" },
    render: { type: "boolean" as const, short: "r", description: "Render the response" },
    mocksDir: { type: "string" as const, short: "d", description: "Mock collections directory", env: "MOCKS_DIR" },
    mockFile: { type: "string" as const, short: "f", description: "Load specific mock file" },
    json: { type: "boolean" as const, short: "j", description: "Output as JSON" },
  },
  async run({ args }) {
    const headers: Record<string, string> = {};
    if (args.header) {
      for (const h of Array.isArray(args.header) ? args.header : [args.header]) {
        const [name, ...rest] = h.split(":");
        if (name && rest.length > 0) headers[name.trim()] = rest.join(":").trim();
      }
    }

    const result = await services.testMatch({
      method: args.method,
      path: args.path,
      query: args.query,
      headers: Object.keys(headers).length > 0 ? headers : undefined,
      body: args.body,
      render: args.render,
      mocksDir: args.mocksDir,
      mockFile: args.mockFile,
    });

    if (args.json) {
      console.log(JSON.stringify(result, null, 2));
      return;
    }

    if (!result.matched) {
      console.log("No matching mock found");
      process.exit(1);
    }

    console.log(`Matched: ${result.mock_id} (priority: ${result.priority})`);
    if (Object.keys(result.captures || {}).length > 0) {
      console.log(`Captures: ${JSON.stringify(result.captures)}`);
    }
    if (result.response) {
      console.log(`\nResponse (${result.response.status}):`);
      console.log(result.response.body);
    }
  },
});
