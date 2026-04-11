/**
 * MSW-compatible utility functions.
 *
 * These mirror MSW's `delay()`, `passthrough()`, and `bypass()` APIs
 * so mockpit can be a drop-in replacement.
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
 *   return MockResponse.json({ ok: true })
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
      : Math.floor(Math.random() * 300) + 100; // 'real': 100-400ms

  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Sentinel value returned by `passthrough()`. */
const PASSTHROUGH_SYMBOL = Symbol.for("mockpit.passthrough");

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
 *   return MockResponse.json({ mocked: true })
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
 * Create a request that bypasses mockpit interception.
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
 *   return MockResponse.json({ ...data, proxied: true })
 * })
 * ```
 */
export function bypass(
  input: string | URL | Request,
  init?: RequestInit
): Request {
  const request = new Request(input, init);
  request.headers.set("x-mockpit-bypass", "1");
  return request;
}

/** Header name used to mark bypassed requests. */
export const BYPASS_HEADER = "x-mockpit-bypass";

/** Header name used to signal network errors. */
export const NETWORK_ERROR_HEADER = "x-mockpit-network-error";
