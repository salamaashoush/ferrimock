/**
 * MSW-compatible HttpResponse: a real Response subclass, so handlers can
 * return it (or any Response) and code that inspects `.status`,
 * `.headers`, `await .json()` keeps working.
 *
 * The registration layer converts returned Response objects into the
 * plain shape the native engine consumes; the native `HttpResponse`
 * namespace in @mockpit/node builds that shape directly and skips the
 * conversion (fastest path, same names and call signatures).
 */

type HttpResponseInit = ResponseInit;

/**
 * A Response whose `json()` resolves to a known body type (MSW's
 * StrictResponse) — makes `HttpResponse.json<T>()` compile-time-checkable
 * against a typed handler.
 */
export interface StrictResponse<BodyType> extends Response {
  json(): Promise<BodyType>;
}

function withContentType(
  init: HttpResponseInit | undefined,
  contentType: string
): HttpResponseInit {
  const headers = new Headers(init?.headers);
  if (!headers.has("content-type")) {
    headers.set("content-type", contentType);
  }
  return { ...init, headers };
}

export class HttpResponse extends Response {
  static json<BodyType>(
    body: BodyType,
    init?: HttpResponseInit
  ): StrictResponse<BodyType> {
    return new HttpResponse(
      JSON.stringify(body),
      withContentType(init, "application/json")
    ) as StrictResponse<BodyType>;
  }

  static text<BodyType extends string>(
    body: BodyType,
    init?: HttpResponseInit
  ): StrictResponse<BodyType> {
    return new HttpResponse(
      body,
      withContentType(init, "text/plain")
    ) as StrictResponse<BodyType>;
  }

  static xml(body: string, init?: HttpResponseInit): HttpResponse {
    return new HttpResponse(body, withContentType(init, "text/xml"));
  }

  static html(body: string, init?: HttpResponseInit): HttpResponse {
    return new HttpResponse(body, withContentType(init, "text/html"));
  }

  static arrayBuffer(
    body: ArrayBuffer | ArrayBufferView,
    init?: HttpResponseInit
  ): HttpResponse {
    return new HttpResponse(
      body as BodyInit,
      withContentType(init, "application/octet-stream")
    );
  }

  static formData(data: FormData, init?: HttpResponseInit): HttpResponse {
    // The Response constructor sets the multipart boundary content-type.
    return new HttpResponse(data, init);
  }

  static redirect(url: string, status = 302): Response {
    return Response.redirect(url, status);
  }

  static override error(): Response {
    return Response.error();
  }
}
