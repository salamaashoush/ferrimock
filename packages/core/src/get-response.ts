/**
 * Standalone request resolution against an ad-hoc handler list — MSW's
 * `getResponse()` / `handleRequest()` without a running server.
 */

import { FerrimockServer } from "@ferrimock/node";
import {
  toEngineHandlers,
  toResponse,
  type AnyHandler,
  type UnhandledRequestStrategy,
} from "./interceptor.js";
import { NETWORK_ERROR_HEADER, PASSTHROUGH_HEADER } from "./msw-compat.js";

/** HTTP method strings accepted by `http.*` handlers (MSW's enum). */
export const HttpMethods = {
  HEAD: "HEAD",
  GET: "GET",
  POST: "POST",
  PUT: "PUT",
  PATCH: "PATCH",
  OPTIONS: "OPTIONS",
  DELETE: "DELETE",
} as const;

export type HttpMethods = (typeof HttpMethods)[keyof typeof HttpMethods];

/**
 * Resolve a `Request` against an ad-hoc handler list without a server
 * (MSW's `getResponse()`). Returns the mocked `Response`, or `undefined`
 * when nothing matched or a handler called `passthrough()`.
 */
export async function getResponse(
  handlers: AnyHandler[],
  request: Request
): Promise<Response | undefined> {
  const server = new FerrimockServer();
  const engineHandlers = toEngineHandlers(handlers);
  if (engineHandlers.length > 0) {
    server.useHandlers(engineHandlers);
  }

  const url = new URL(request.url);
  const query = url.search ? url.search.slice(1) : undefined;

  let body: Uint8Array | undefined;
  if (request.method !== "GET" && request.method !== "HEAD") {
    try {
      const bytes = new Uint8Array(await request.clone().arrayBuffer());
      if (bytes.length > 0) body = bytes;
    } catch {}
  }

  const headers: Record<string, string> = { host: url.host };
  request.headers.forEach((value, key) => {
    headers[key] = value;
  });

  let excludeIds: string[] | null = null;
  for (;;) {
    const match = await server.matchRequest(
      request.method,
      url.pathname,
      query ?? null,
      headers,
      body ?? null,
      null,
      excludeIds
    );
    if (!match) {
      return undefined;
    }
    if (match.fallthrough) {
      (excludeIds ??= []).push(match.mockId);
      continue;
    }
    if (match.headers[PASSTHROUGH_HEADER] === "1") {
      return undefined;
    }
    if (match.headers[NETWORK_ERROR_HEADER] === "1") {
      return Response.error();
    }
    return toResponse(match);
  }
}

/** Lifecycle emitter surface `handleRequest` reports into. */
interface HandleRequestEmitter {
  emit(event: string, payload: { request: Request; requestId: string; response?: Response }): void;
}

export interface HandleRequestOptions {
  /** Called when the request has no mocked response and should hit the network. */
  onPassthroughResponse?(request: Request): void;
  /** Called with the mocked response before it is returned. */
  onMockedResponse?(response: Response): void;
}

/**
 * Resolve a request against handlers with lifecycle reporting (MSW's
 * `handleRequest()`). Applies the `onUnhandledRequest` strategy when no
 * handler responds and returns the mocked `Response`, or `undefined`
 * for passthrough/unhandled requests.
 */
export async function handleRequest(
  request: Request,
  requestId: string,
  handlers: AnyHandler[],
  options?: { onUnhandledRequest?: UnhandledRequestStrategy },
  emitter?: HandleRequestEmitter,
  handleRequestOptions?: HandleRequestOptions
): Promise<Response | undefined> {
  emitter?.emit("request:start", { request, requestId });

  const response = await getResponse(handlers, request);

  if (!response) {
    reportUnhandled(request, options?.onUnhandledRequest ?? "warn");
    emitter?.emit("request:unhandled", { request, requestId });
    emitter?.emit("request:end", { request, requestId });
    handleRequestOptions?.onPassthroughResponse?.(request);
    return undefined;
  }

  emitter?.emit("request:match", { request, requestId });
  emitter?.emit("request:end", { request, requestId });
  handleRequestOptions?.onMockedResponse?.(response);
  return response;
}

function reportUnhandled(
  request: Request,
  strategy: UnhandledRequestStrategy
): void {
  const msg = `[ferrimock] Unhandled ${request.method} ${request.url}`;
  if (strategy === "bypass") return;
  if (strategy === "warn") {
    console.warn(msg);
    return;
  }
  if (strategy === "error") {
    throw new Error(msg);
  }
  if (typeof strategy === "function") {
    strategy(request, {
      warning() {
        console.warn(msg);
      },
      error() {
        throw new Error(msg);
      },
    });
  }
}
