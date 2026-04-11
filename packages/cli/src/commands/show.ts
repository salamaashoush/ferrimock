import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const show = defineCommand({
  meta: { name: "show", description: "Show mock definition details", aliases: ["s"] },
  args: {
    mockId: { type: "positional" as const, required: true, description: "Mock ID to display" },
    mocksDir: { type: "string" as const, short: "d", description: "Mock collections directory", env: "MOCKS_DIR" },
  },
  async run({ args }) {
    const mock = await services.show(args.mockId, args.mocksDir);
    if (!mock) {
      console.error(`Mock not found: ${args.mockId}`);
      process.exit(1);
    }
    console.log(JSON.stringify(mock, null, 2));
  },
});
