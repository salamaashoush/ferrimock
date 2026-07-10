/**
 * MSW-compatible utility functions.
 *
 * These mirror MSW's `delay()`, `passthrough()`, and `bypass()` APIs
 * so ferrimock can be a drop-in replacement.
 */

/**
 * Delay the response by a specified duration.
 *
 * Use inside handler functions to simulate network latency.
 *
 * @param durationOrMode - Milliseconds to delay, or:
 *   - `'real'` (default): random 100-400ms delay simulating real network
 *   - `'infinite'`: delay indefinitely (useful for timeout testing)
 *   - number: exact milliseconds
 *
 * @example
 * ```ts
 * http.get('/api/data', async () => {
 *   await delay(200)
 *   return HttpResponse.json({ ok: true })
 * })
 * ```
 */
export function delay(durationOrMode?: number | "real" | "infinite"): Promise<void> {
  if (durationOrMode === "infinite") {
    return new Promise(() => {}); // never resolves
  }

  const ms =
    typeof durationOrMode === "number"
      ? durationOrMode
      : Math.floor(Math.random() * 301) + 100; // 'real': 100-400ms

  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Sentinel value returned by `passthrough()`. */
const PASSTHROUGH_SYMBOL = Symbol.for("ferrimock.passthrough");

/**
 * Signal that the request should pass through to the actual network.
 *
 * Return this from a handler to skip mocking and forward the request
 * to the real server. Equivalent to MSW's `passthrough()`.
 *
 * @example
 * ```ts
 * http.get('/api/data', async ({ request }) => {
 *   if (request.headers.get('x-real') === '1') {
 *     return passthrough()
 *   }
 *   return HttpResponse.json({ mocked: true })
 * })
 * ```
 */
export function passthrough(): any {
  return { [PASSTHROUGH_SYMBOL]: true };
}

/** Check if a value is a passthrough sentinel. */
export function isPassthrough(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    PASSTHROUGH_SYMBOL in value
  );
}

/**
 * Create a request that bypasses ferrimock interception.
 *
 * Useful for making real network requests from inside handlers
 * without triggering the mock interceptor.
 *
 * @param input - URL string, URL object, or Request to bypass.
 * @param init - Optional RequestInit for the bypassed request.
 * @returns A Request with a bypass marker header.
 *
 * @example
 * ```ts
 * http.get('/api/proxy', async ({ request }) => {
 *   const realResponse = await fetch(bypass(request))
 *   const data = await realResponse.json()
 *   return HttpResponse.json({ ...data, proxied: true })
 * })
 * ```
 */
export function bypass(
  input: string | URL | Request,
  init?: RequestInit
): Request {
  const request = new Request(input, init);
  request.headers.set("x-ferrimock-bypass", "1");
  return request;
}

/** Path parameters extracted by `matchRequestUrl`. */
export type PathParams = Record<string, string | string[]>;

/** Strip the query string and hash from a path (MSW's `cleanUrl`). */
export function cleanUrl(path: string): string {
  return path.replace(/[?#].*$/, "");
}

/**
 * Compile an MSW-style path (`:param` segments, `:param+`/`:param*`
 * repeatable modifiers, full-segment `*` wildcards) to a RegExp — same
 * semantics as the native engine's path patterns. Wildcards capture into
 * numeric params keys; repeatable params come back as `string[]`.
 */
function pathToRegex(path: string): RegExp {
  let wildcardIndex = 0;
  let pattern = "";
  path.split("/").forEach((segment, index) => {
    const sep = index === 0 ? "" : "/";
    if (segment.startsWith(":") && segment.length > 1) {
      const param = segment.slice(1);
      if (param.length > 1 && param.endsWith("+")) {
        pattern += `${sep}(?<__rp${param.slice(0, -1)}>.+)`;
      } else if (param.length > 1 && param.endsWith("*")) {
        // Zero segments must also match without the separator.
        pattern += `(?:${sep}(?<__rp${param.slice(0, -1)}>.*))?`;
      } else {
        pattern += `${sep}(?<${param}>[^/]+)`;
      }
    } else if (segment === "*") {
      pattern += `${sep}(?<__wc${wildcardIndex++}>.*)`;
    } else {
      pattern += sep + segment.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    }
  });
  return new RegExp(`^${pattern}$`);
}

function decodeParam(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function groupsToParams(
  groups: Record<string, string | undefined> | undefined
): PathParams {
  const params: PathParams = {};
  for (const [name, value] of Object.entries(groups ?? {})) {
    if (value === undefined) continue;
    if (name.startsWith("__rp") && name.length > 4) {
      // Zero-segment `:name*` matches empty — MSW omits the param.
      if (value === "") continue;
      params[name.slice(4)] = value.split("/").map(decodeParam);
      continue;
    }
    const key = /^__wc\d+$/.test(name) ? name.slice(4) : name;
    params[key] = decodeParam(value);
  }
  return params;
}

/**
 * Match a URL against an MSW-style path predicate (MSW's
 * `matchRequestUrl`). Returns whether it matched and any extracted
 * path parameters.
 *
 * @param url - The request URL.
 * @param path - String path (`/users/:id`, absolute URL, `*` wildcards)
 *   or RegExp (tested against the pathname).
 * @param baseUrl - Base for resolving a relative string path's host
 *   (defaults to matching any host).
 */
export function matchRequestUrl(
  url: URL,
  path: string | RegExp,
  baseUrl?: string
): { matches: boolean; params: PathParams } {
  if (path instanceof RegExp) {
    const match = path.exec(url.pathname);
    return {
      matches: match !== null,
      params: groupsToParams(match?.groups),
    };
  }

  let pathname = path;
  let expectedHost: string | undefined;
  const absolute = /^(?:https?|wss?):\/\//.exec(path);
  if (absolute) {
    const rest = path.slice(absolute[0].length);
    const slash = rest.indexOf("/");
    expectedHost = slash === -1 ? rest : rest.slice(0, slash);
    pathname = slash === -1 ? "/" : rest.slice(slash);
  } else if (baseUrl) {
    expectedHost = new URL(baseUrl).host;
  }

  if (expectedHost && expectedHost !== "*" && url.host !== expectedHost) {
    return { matches: false, params: {} };
  }

  if (pathname === "*") {
    return { matches: true, params: { "0": url.pathname } };
  }

  const match = pathToRegex(cleanUrl(pathname)).exec(url.pathname);
  return {
    matches: match !== null,
    params: groupsToParams(match?.groups),
  };
}

/** Header name used to mark bypassed requests. */
export const BYPASS_HEADER = "x-ferrimock-bypass";

/** Header name used to signal network errors. */
export const NETWORK_ERROR_HEADER = "x-ferrimock-network-error";

/** Marker header: handler called passthrough() — perform the real request. */
export const PASSTHROUGH_HEADER = "x-ferrimock-passthrough";

/** Marker header: handler returned undefined — retry matching without this mock. */
export const FALLTHROUGH_HEADER = "x-ferrimock-fallthrough";

/** Marker header: the original Response is stashed under this token. */
export const STREAM_ID_HEADER = "x-ferrimock-stream-id";
