import { http, HttpResponse } from "ferrimock";

export default [
  http.get("/api/exported", () => HttpResponse.json({ style: "export-default" })),
];
