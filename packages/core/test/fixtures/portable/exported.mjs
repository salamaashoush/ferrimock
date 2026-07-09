import { http, HttpResponse } from "mockpit";

export default [
  http.get("/api/exported", () => HttpResponse.json({ style: "export-default" })),
];
