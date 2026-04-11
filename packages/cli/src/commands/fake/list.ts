import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const fakeList = defineCommand({
  meta: { name: "list", description: "List available fake data generators", aliases: ["ls"] },
  args: {
    category: { type: "string" as const, short: "c", description: "Filter by category" },
    search: { type: "string" as const, short: "s", description: "Search by name or description" },
    json: { type: "boolean" as const, short: "j", description: "Output as JSON" },
  },
  run({ args }) {
    const generators = services.listGenerators(args.category, args.search);

    if (args.json) {
      console.log(JSON.stringify(generators, null, 2));
      return;
    }

    if (generators.length === 0) {
      console.log("No generators found");
      return;
    }

    // Group by category
    const grouped = new Map<string, typeof generators>();
    for (const g of generators) {
      const list = grouped.get(g.category) ?? [];
      list.push(g);
      grouped.set(g.category, list);
    }

    for (const [category, gens] of grouped) {
      console.log(`\n${category}:`);
      for (const g of gens) {
        console.log(`  ${g.name.padEnd(20)} ${g.description} (e.g. ${g.example})`);
      }
    }
    console.log(`\nTotal: ${generators.length} generators`);
  },
});
