/**
 * Side channel carrying handler Response objects past the engine.
 *
 * When the interceptor is active, a handler's Response never needs its
 * body to cross the NAPI boundary: the engine gets status/headers for
 * matching and events, and the original Response (including a live
 * ReadableStream body) is delivered to the caller from here, keyed by a
 * token in the marker header. Keyed on globalThis so jiti's second
 * module graph shares the same stash.
 */

const STASH_SLOT = Symbol.for("mockpit.streamStash");
const ACTIVE_SLOT = Symbol.for("mockpit.interceptorActive");

interface StashEntry {
  response: Response;
  at: number;
}

const STASH_TTL_MS = 60_000;

function stash(): Map<string, StashEntry> {
  const g = globalThis as Record<PropertyKey, unknown>;
  if (!g[STASH_SLOT]) {
    g[STASH_SLOT] = new Map<string, StashEntry>();
  }
  return g[STASH_SLOT] as Map<string, StashEntry>;
}

export function stashResponse(response: Response): string {
  const map = stash();
  const now = Date.now();
  // Drop entries the interceptor never consumed (redirect follows,
  // dropped matches) so the stash cannot grow unbounded.
  for (const [key, entry] of map) {
    if (now - entry.at > STASH_TTL_MS) {
      map.delete(key);
    }
  }
  const token = crypto.randomUUID();
  map.set(token, { response, at: now });
  return token;
}

export function takeResponse(token: string | undefined): Response | undefined {
  if (!token) return undefined;
  const map = stash();
  const entry = map.get(token);
  if (entry) {
    map.delete(token);
    return entry.response;
  }
  return undefined;
}

export function setInterceptorActive(active: boolean): void {
  (globalThis as Record<PropertyKey, unknown>)[ACTIVE_SLOT] = active;
}

export function isInterceptorActive(): boolean {
  return (globalThis as Record<PropertyKey, unknown>)[ACTIVE_SLOT] === true;
}
