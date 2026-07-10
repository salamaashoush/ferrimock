/**
 * MSW-compatible Node entry point: `setupServer` over the interceptor.
 *
 * ```ts
 * import { setupServer } from "ferrimock/node";
 * import { http, HttpResponse } from "ferrimock";
 *
 * const server = setupServer(
 *   http.get("/api/user", () => HttpResponse.json({ name: "John" }))
 * );
 * server.listen({ onUnhandledRequest: "error" });
 * ```
 */

import {
  FerrimockInterceptor,
  type AnyHandler,
  type ApplyOptions,
  type ListedHandler,
  type UnhandledRequestStrategy,
} from "./interceptor.js";
import type { LifecycleEvents } from "./events.js";

export interface ListenOptions {
  onUnhandledRequest?: UnhandledRequestStrategy;
}

export interface SetupServerApi {
  listen(options?: ListenOptions): void;
  close(): void;
  use(...handlers: AnyHandler[]): void;
  resetHandlers(...nextHandlers: AnyHandler[]): void;
  restoreHandlers(): void;
  listHandlers(): ListedHandler[];
  boundary<Args extends unknown[], R>(
    callback: (...args: Args) => R
  ): (...args: Args) => R;
  events: LifecycleEvents;
}

/**
 * Create an MSW-compatible mock server for Node.
 *
 * `listen()` patches fetch, XMLHttpRequest, and http.ClientRequest;
 * `close()` restores them. Defaults `onUnhandledRequest` to `"warn"`
 * (MSW's default); pass `"bypass"` to silence it.
 */
export function setupServer(...handlers: AnyHandler[]): SetupServerApi {
  const interceptor = new FerrimockInterceptor();
  if (handlers.length > 0) {
    interceptor.useHandlers(handlers);
  }

  return {
    listen(options?: ListenOptions): void {
      const applyOptions: ApplyOptions = {
        onUnhandledRequest: options?.onUnhandledRequest ?? "warn",
      };
      interceptor.apply(applyOptions);
    },
    close(): void {
      interceptor.dispose();
    },
    use(...runtimeHandlers: AnyHandler[]): void {
      interceptor.use(...runtimeHandlers);
    },
    resetHandlers(...nextHandlers: AnyHandler[]): void {
      interceptor.resetHandlers(...nextHandlers);
    },
    restoreHandlers(): void {
      interceptor.restoreHandlers();
    },
    listHandlers() {
      return interceptor.listHandlers();
    },
    boundary(callback) {
      return interceptor.boundary(callback);
    },
    events: interceptor.events,
  };
}
