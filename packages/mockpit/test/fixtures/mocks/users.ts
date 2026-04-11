import { http, MockResponse } from "@mockpit/node";

export default [
  http.get("/api/users/:id", async ({ params }) => {
    return MockResponse.json({ id: params.id, name: "John", source: "ts-handler" });
  }),

  http.post("/api/users", async ({ bodyJson }) => {
    return MockResponse.json(
      { id: "new-1", ...bodyJson, source: "ts-handler" },
      { status: 201 }
    );
  }),
];
