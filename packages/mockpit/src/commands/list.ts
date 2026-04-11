import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const list = defineCommand({
  meta: { name: "list", description: "List mock definitions", aliases: ["ls"] },
  args: {
    mocksDir: { type: "string" as const, short: "d", description: "Mock collections directory", env: "MOCKS_DIR" },
    filter: { type: "string" as const, short: "c", description: "Filter by collection/ID" },
    json: { type: "boolean" as const, short: "j", description: "Output as JSON" },
  },
  async run({ args }) {
    const result = await services.list({ mocksDir: args.mocksDir, filter: args.filter });

    if (args.json) {
      console.log(JSON.stringify(result, null, 2));
      return;
    }

    if (result.total === 0) {
      console.log("No mocks found");
      return;
    }

    console.log(`Found ${result.total} mock(s):\n`);
    const pad = (s: string, n: number) => s.padEnd(n);
    console.log(`${pad("ID", 30)} ${pad("Methods", 10)} ${pad("Status", 8)} Priority`);
    console.log("-".repeat(65));
    for (const m of result.mocks) {
      const methods = m.methods.length > 0 ? m.methods.join(",") : "ANY";
      console.log(`${pad(m.id, 30)} ${pad(methods, 10)} ${pad(String(m.status), 8)} ${m.priority}`);
    }
  },
});
