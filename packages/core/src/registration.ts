/**
 * Handler factories: MSW-compatible resolver semantics over the native
 * `http`/`graphql` bindings, plus side-effect collection so CLI-style
 * mock files (bare `http.get(...)` calls, no export) register under Node
 * exactly like they do on the embedded QuickJS runtime.
 *
 * The wrapper normalizes what crosses the NAPI boundary:
 * - sync resolvers become async (the NAPI server lane types handlers as
 *   Promise-returning; a bare return fails its PromiseRaw::then)
 * - a returned Response/HttpResponse converts to the plain engine shape
 * - `passthrough()` sentinels become the passthrough marker header
 * - generator resolvers advance per request, last value repeats (MSW)
 * - undefined/null returns stay null = fall-through to the next mock
 */

import {
  http as nativeHttp,
  graphql as nativeGraphql,
  type RequestHandler,
  type HandlerResponse,
} from "@mockpit/node";
import {
  isPassthrough,
  NETWORK_ERROR_HEADER,
  PASSTHROUGH_HEADER,
  STREAM_ID_HEADER,
} from "./msw-compat.js";
import { isInterceptorActive, stashResponse } from "./stream-stash.js";

// Keyed on globalThis: mock files may be loaded through a second module
// graph (jiti), whose copy of this module must see the same window.
const COLLECT_SLOT = Symbol.for("mockpit.collecting");

/** JS-only handlers (WebSocket) share the collection window with the
 * engine's RequestHandlers; the loader splits them by `kind` tag. */
type CollectedHandler = RequestHandler | { kind: "websocket" };

type CollectSlot = { handlers: CollectedHandler[] | null };

function slot(): CollectSlot {
  const g = globalThis as Record<PropertyKey, unknown>;
  if (!g[COLLECT_SLOT]) {
    g[COLLECT_SLOT] = { handlers: null } satisfies CollectSlot;
  }
  return g[COLLECT_SLOT] as CollectSlot;
}

/**
 * Push a handler into the open collection window (used by the `ws`
 * factory; `http`/`graphql` collect through wrapNamespace).
 * @internal
 */
export function registerCollected(handler: CollectedHandler): void {
  slot().handlers?.push(handler);
}

async function responseToEngine(
  response: Response,
  forceBuffer: boolean
): Promise<HandlerResponse> {
  if (response.type === "error") {
    return { status: 0, headers: { [NETWORK_ERROR_HEADER]: "1" } };
  }
  const headers: Record<string, string> = {};
  response.headers.forEach((value, key) => {
    if (key !== "set-cookie") {
      headers[key] = value;
    }
  });
  // Multiple Set-Cookie values survive as a newline-joined single entry;
  // the interceptor and the standalone server split them back out.
  const setCookies = response.headers.getSetCookie?.() ?? [];
  if (setCookies.length > 0) {
    headers["set-cookie"] = setCookies.join("\n");
  }

  // Interceptor lane: keep the original Response (streams stay live and
  // the body never round-trips through the engine); only status/headers
  // cross for matching. The standalone TCP server buffers instead.
  if (!forceBuffer && isInterceptorActive()) {
    headers[STREAM_ID_HEADER] = stashResponse(response);
    return {
      status: response.status,
      statusText: response.statusText || undefined,
      headers,
    };
  }

  return {
    status: response.status,
    statusText: response.statusText || undefined,
    headers,
    bodyBytes: new Uint8Array(await response.arrayBuffer()),
  };
}

/** @internal Shared with the sse() factory, which builds its own
 * Response instead of going through wrapResolver. */
export async function normalizeResult(
  result: unknown,
  forceBuffer = false
): Promise<HandlerResponse | null> {
  if (result === undefined || result === null) {
    return null; // fall-through
  }
  if (isPassthrough(result)) {
    return { headers: { [PASSTHROUGH_HEADER]: "1" } };
  }
  if (result instanceof Response) {
    return responseToEngine(result, forceBuffer);
  }
  return result as HandlerResponse;
}

type Resolver = (info: unknown) => unknown;

function isGeneratorFunction(fn: Resolver): boolean {
  const name = fn?.constructor?.name;
  return name === "GeneratorFunction" || name === "AsyncGeneratorFunction";
}

function wrapResolver(resolver: Resolver): Resolver {
  if (isGeneratorFunction(resolver)) {
    let iterator: Iterator<unknown> | AsyncIterator<unknown> | null = null;
    // Stored post-normalization: a Response body can only be read once,
    // so the repeated "last value" must be the converted shape.
    let lastValue: HandlerResponse | null = null;
    return async (info: unknown) => {
      iterator ??= (resolver as (i: unknown) => Iterator<unknown>)(info);
      const step = await iterator.next();
      if (step.value !== undefined) {
        // Buffered even on the interceptor lane: the last value repeats,
        // and a stashed Response delivers only once.
        lastValue = await normalizeResult(step.value, true);
        return lastValue;
      }
      return step.done ? lastValue : null;
    };
  }
  return async (info: unknown) => normalizeResult(await resolver(info));
}

function wrapNamespace<T extends object>(ns: T): T {
  const wrapped: Record<string, unknown> = {};
  for (const key of Object.keys(ns)) {
    const member = (ns as Record<string, unknown>)[key];
    if (typeof member !== "function") {
      wrapped[key] = member;
      continue;
    }
    wrapped[key] = (...args: unknown[]) => {
      const normalized = args.map((arg) =>
        typeof arg === "function" ? wrapResolver(arg as Resolver) : arg
      );
      const handler = (member as (...a: unknown[]) => RequestHandler)(...normalized);
      slot().handlers?.push(handler);
      return handler;
    };
  }
  return wrapped as T;
}

/** `http.*` factories that also record into an open collection window. */
export const http: typeof nativeHttp = wrapNamespace(nativeHttp);

type GraphQLResolver = Parameters<typeof nativeGraphql.query>[1];
type GraphQLOptions = Parameters<typeof nativeGraphql.query>[2];

interface GraphQLLink {
  query(
    operationName: string | RegExp,
    resolver: GraphQLResolver,
    options?: GraphQLOptions
  ): RequestHandler;
  mutation(
    operationName: string | RegExp,
    resolver: GraphQLResolver,
    options?: GraphQLOptions
  ): RequestHandler;
  operation(resolver: GraphQLResolver, options?: GraphQLOptions): RequestHandler;
}

const wrappedGraphql = wrapNamespace(nativeGraphql);

/**
 * `graphql.*` factories that also record into an open collection window,
 * plus `graphql.link(url)` for endpoint-scoped handlers (the endpoint
 * becomes a native matcher on the mock, not a resolver-side check).
 */
export const graphql: typeof nativeGraphql & {
  link(url: string): GraphQLLink;
} = {
  ...wrappedGraphql,
  link(url: string): GraphQLLink {
    return {
      query: (operationName, resolver, options) =>
        wrappedGraphql.query(operationName, resolver, options, url),
      mutation: (operationName, resolver, options) =>
        wrappedGraphql.mutation(operationName, resolver, options, url),
      operation: (resolver, options) =>
        wrappedGraphql.operation(resolver, options, url),
    };
  },
};

/**
 * Run `fn` (typically a dynamic import of a mock file) with a
 * collection window open and return whatever handlers the factories
 * built during it. Windows do not nest and imports are expected to run
 * sequentially (loadMocksDir imports one file at a time).
 *
 * `handlers` carries the engine's RequestHandlers (existing callers'
 * meaning is unchanged); JS-only WebSocket handlers come back in
 * `wsHandlers`.
 */
export async function collectHandlers<T>(
  fn: () => Promise<T>
): Promise<{
  result: T;
  handlers: RequestHandler[];
  wsHandlers: Array<{ kind: "websocket" }>;
}> {
  const s = slot();
  const previous = s.handlers;
  const bucket: CollectedHandler[] = [];
  s.handlers = bucket;
  try {
    const result = await fn();
    const handlers: RequestHandler[] = [];
    const wsHandlers: Array<{ kind: "websocket" }> = [];
    for (const handler of bucket) {
      if ((handler as { kind?: string }).kind === "websocket") {
        wsHandlers.push(handler as { kind: "websocket" });
      } else {
        handlers.push(handler as RequestHandler);
      }
    }
    return { result, handlers, wsHandlers };
  } finally {
    s.handlers = previous;
  }
}
