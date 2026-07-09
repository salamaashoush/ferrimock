/**
 * Virtual cookie jar (MSW parity): Set-Cookie headers on mocked
 * responses are remembered per host and merged into subsequent
 * requests' Cookie header, so resolver `cookies` behave like a
 * browser's document.cookie. Process-global, like MSW's store —
 * it survives server close()/re-listen within one process.
 *
 * Minimal semantics: name=value per host; attributes (Path, Domain,
 * Expires, Max-Age) are ignored.
 */

const store = new Map<string, Map<string, string>>();

function parsePair(pair: string): [string, string] | null {
  const eq = pair.indexOf("=");
  if (eq === -1) return null;
  const name = pair.slice(0, eq).trim();
  if (!name) return null;
  return [name, pair.slice(eq + 1).trim()];
}

/** Record cookies from a mocked response's Set-Cookie value(s). */
export function storeResponseCookies(
  host: string,
  setCookieValues: string[]
): void {
  for (const line of setCookieValues) {
    const entry = parsePair(line.split(";", 1)[0] ?? "");
    if (!entry) continue;
    let jar = store.get(host);
    if (!jar) {
      jar = new Map();
      store.set(host, jar);
    }
    jar.set(entry[0], entry[1]);
  }
}

/**
 * Merge stored cookies for a host into an outgoing Cookie header.
 * Cookies the request itself carries win over stored ones.
 */
export function mergeStoredCookies(
  host: string,
  existing?: string
): string | undefined {
  const jar = store.get(host);
  if (!jar || jar.size === 0) return existing;
  const merged = new Map(jar);
  if (existing) {
    for (const pair of existing.split(";")) {
      const entry = parsePair(pair);
      if (entry) merged.set(entry[0], entry[1]);
    }
  }
  return [...merged].map(([k, v]) => `${k}=${v}`).join("; ");
}

export function clearCookieStore(): void {
  store.clear();
}
