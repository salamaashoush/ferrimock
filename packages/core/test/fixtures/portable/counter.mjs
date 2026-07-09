import { http, HttpResponse, fake } from "mockpit";

let count = 0;

http.post("/api/count", () => {
  count += 1;
  return HttpResponse.json({ count });
});

http.get("/api/id", () => HttpResponse.json({ id: fake.uuid() }));
