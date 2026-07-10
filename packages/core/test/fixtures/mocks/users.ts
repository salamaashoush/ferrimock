import { http, HttpResponse } from "@ferrimock/node";

export default [
  http.get("/api/users/:id", async ({ params }) => {
    return HttpResponse.json({ id: params.id, name: "John", source: "ts-handler" });
  }),

  http.post("/api/users", async ({ request }) => {
    const body = await request.json();
    return HttpResponse.json(
      { id: "new-1", ...body, source: "ts-handler" },
      { status: 201 }
    );
  }),
];
