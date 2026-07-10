/**
 * MSW-compatible `ws` namespace over the native engine.
 *
 * `ws.link(url).addEventListener("connection", listener)` builds a real
 * engine mock (`ws.handler` NAPI): registration, matching, and lifecycle
 * (use/resetHandlers/boundary/listHandlers) flow through the engine
 * exactly like `http` handlers. This module is only socket plumbing:
 *
 * - Interceptor lane: `@mswjs/interceptors` patches globalThis.WebSocket
 *   and owns the sockets; the interceptor resolves WHICH handlers match
 *   via `matchWsConnections` and this module dispatches the `connection`
 *   listeners with the interceptor's `{ client, server }` objects.
 * - TCP lane (`FerrimockServer.listen()`): the Rust connection driver owns
 *   the socket and upstream forwarding; events arrive through the
 *   registered dispatch callback and sends go back over native handles.
 */

import {
  ws as nativeWs,
  type RequestHandler,
  type WebSocketClientHandle,
  type WebSocketServerHandle,
} from "ferrimock-node";
import type {
  WebSocketConnectionData,
  WebSocketData,
  WebSocketClientConnectionProtocol,
} from "@mswjs/interceptors/WebSocket";
import type { PathParams } from "./msw-compat.js";
import { registerCollected } from "./registration.js";

export type WebSocketConnection = WebSocketConnectionData & {
  params: PathParams;
};

export type WebSocketConnectionListener = (
  connection: WebSocketConnection
) => void;

const kPropagationStoppedAt = Symbol("kPropagationStoppedAt");

/** Handlers by engine mock id, so an intercepted connection's engine
 * matches resolve back to their JS listeners. */
const handlersByMockId = new Map<string, WebSocketHandler>();

/** @internal */
export function getWsHandler(mockId: string): WebSocketHandler | undefined {
  return handlersByMockId.get(mockId);
}

/** @internal Prune dispatch entries whose engine mocks are gone. */
export function pruneWsHandlers(liveMockIds: Set<string>): void {
  for (const id of handlersByMockId.keys()) {
    if (!liveMockIds.has(id)) {
      handlersByMockId.delete(id);
    }
  }
}

/**
 * Cross-handler stop-propagation (ported from MSW): each handler wraps
 * the connection's event targets so `stopPropagation()` in one
 * handler's listener suppresses the OTHER handlers' listeners for that
 * event, while listeners registered by the same handler still run.
 */
function attachStopPropagation(
  handler: WebSocketHandler,
  target: {
    addEventListener: (type: any, listener: any, options?: any) => void;
  }
): void {
  const original = target.addEventListener.bind(target);
  target.addEventListener = (type: any, listener: any, options?: any) => {
    const wrapped = (event: any, ...rest: any[]) => {
      const stoppedAt = event[kPropagationStoppedAt] as
        | WebSocketHandler
        | undefined;
      if (stoppedAt && stoppedAt !== handler) {
        return;
      }
      const originalStop = event.stopPropagation?.bind(event);
      const originalStopImmediate =
        event.stopImmediatePropagation?.bind(event);
      if (originalStop) {
        event.stopPropagation = () => {
          event[kPropagationStoppedAt] ??= handler;
          originalStop();
        };
      }
      if (originalStopImmediate) {
        event.stopImmediatePropagation = () => {
          event[kPropagationStoppedAt] ??= handler;
          originalStopImmediate();
        };
      }
      return listener(event, ...rest);
    };
    original(type, wrapped, options);
  };
}

function toNativeData(data: WebSocketData): string | Uint8Array | Promise<Uint8Array> {
  if (typeof data === "string") {
    return data;
  }
  if (data instanceof Blob) {
    return data.arrayBuffer().then((buffer) => new Uint8Array(buffer));
  }
  if (data instanceof ArrayBuffer) {
    return new Uint8Array(data);
  }
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

type BridgeEvent = {
  type:
    | "connection"
    | "message"
    | "close"
    | "server-open"
    | "server-message"
    | "server-error"
    | "server-close";
  connectionId: string;
  url?: string;
  params?: Record<string, string>;
  protocols?: string[];
  client?: WebSocketClientHandle;
  server?: WebSocketServerHandle;
  data?: string | Uint8Array;
  code?: number;
  reason?: string;
  message?: string;
};

type Listener = (event: Event) => void;

/** Per-connection JS state for the TCP lane. */
class BridgeConnection {
  readonly clientListeners = new Map<string, Listener[]>();
  readonly serverListeners = new Map<string, Listener[]>();

  constructor(
    readonly clientHandle: WebSocketClientHandle,
    readonly serverHandle: WebSocketServerHandle
  ) {}

  private static add(map: Map<string, Listener[]>, type: string, listener: Listener) {
    const bucket = map.get(type);
    if (bucket) {
      bucket.push(listener);
    } else {
      map.set(type, [listener]);
    }
  }

  private static remove(map: Map<string, Listener[]>, type: string, listener: Listener) {
    const bucket = map.get(type);
    if (bucket) {
      const index = bucket.indexOf(listener);
      if (index !== -1) bucket.splice(index, 1);
    }
  }

  /** Dispatch to one bucket; returns `event.defaultPrevented`. */
  dispatch(map: "client" | "server", type: string, event: Event): boolean {
    const buckets = map === "client" ? this.clientListeners : this.serverListeners;
    for (const listener of buckets.get(type) ?? []) {
      listener(event);
    }
    return event.defaultPrevented;
  }

  buildClient(id: string, url: string): WebSocketClientConnectionProtocol {
    const handle = this.clientHandle;
    const listeners = this.clientListeners;
    return {
      id,
      url: new URL(url),
      send(data: WebSocketData) {
        const converted = toNativeData(data);
        if (converted instanceof Promise) {
          void converted.then((bytes) => handle.send(bytes));
        } else {
          handle.send(converted);
        }
      },
      close(code?: number, reason?: string) {
        handle.close(code, reason);
      },
      addEventListener(type: string, listener: any) {
        BridgeConnection.add(listeners, type, listener);
      },
      removeEventListener(type: string, listener: any) {
        BridgeConnection.remove(listeners, type, listener);
      },
    } as unknown as WebSocketClientConnectionProtocol;
  }

  buildServer(): unknown {
    const handle = this.serverHandle;
    const listeners = this.serverListeners;
    return {
      connect() {
        handle.connect();
      },
      send(data: WebSocketData) {
        const converted = toNativeData(data);
        if (converted instanceof Promise) {
          void converted.then((bytes) => handle.send(bytes));
        } else {
          handle.send(converted);
        }
      },
      close() {
        handle.close();
      },
      addEventListener(type: string, listener: any) {
        BridgeConnection.add(listeners, type, listener);
      },
      removeEventListener(type: string, listener: any) {
        BridgeConnection.remove(listeners, type, listener);
      },
    };
  }
}

function messageEvent(type: string, data: unknown): MessageEvent {
  return new MessageEvent(type, { data, cancelable: true });
}

function closeEvent(type: string, code: number, reason: string): Event {
  const event = new Event(type, { cancelable: true });
  Object.defineProperty(event, "code", { value: code, enumerable: true });
  Object.defineProperty(event, "reason", { value: reason, enumerable: true });
  return event;
}

export class WebSocketHandler {
  readonly kind = "websocket" as const;
  /** The engine mock behind this handler; consumed on registration. */
  readonly native: RequestHandler;
  readonly id: string;
  private listeners: WebSocketConnectionListener[] = [];
  /** Live TCP-lane connections by connection id. */
  private connections = new Map<string, BridgeConnection>();

  constructor(readonly url: string | RegExp) {
    this.native = nativeWs.handler(url, (event: unknown) =>
      this.dispatchBridgeEvent(event as BridgeEvent)
    );
    // Read before registration consumes the definition.
    this.id = this.native.id ?? "";
    handlersByMockId.set(this.id, this);
  }

  on(listener: WebSocketConnectionListener): void {
    this.listeners.push(listener);
  }

  /**
   * Interceptor lane: dispatch an intercepted connection (already
   * matched by the engine) to this handler's listeners.
   */
  run(connection: WebSocketConnectionData, params: PathParams): void {
    attachStopPropagation(this, connection.client);
    attachStopPropagation(this, connection.server);

    const enriched: WebSocketConnection = Object.assign(
      Object.create(Object.getPrototypeOf(connection) ?? Object.prototype),
      connection,
      { params }
    );
    for (const listener of this.listeners) {
      listener(enriched);
    }
  }

  /** TCP lane: one Rust driver event for one of this handler's connections. */
  private async dispatchBridgeEvent(event: BridgeEvent): Promise<boolean> {
    switch (event.type) {
      case "connection": {
        const conn = new BridgeConnection(event.client!, event.server!);
        this.connections.set(event.connectionId, conn);
        const connection = {
          client: conn.buildClient(event.connectionId, event.url!),
          server: conn.buildServer(),
          info: { protocols: event.protocols ?? [] },
          params: (event.params ?? {}) as PathParams,
        } as unknown as WebSocketConnection;
        for (const listener of this.listeners) {
          listener(connection);
        }
        return false;
      }
      case "message": {
        const conn = this.connections.get(event.connectionId);
        if (!conn) return false;
        return conn.dispatch("client", "message", messageEvent("message", event.data));
      }
      case "close": {
        const conn = this.connections.get(event.connectionId);
        this.connections.delete(event.connectionId);
        if (!conn) return false;
        return conn.dispatch(
          "client",
          "close",
          closeEvent("close", event.code ?? 1000, event.reason ?? "")
        );
      }
      case "server-open": {
        const conn = this.connections.get(event.connectionId);
        if (!conn) return false;
        return conn.dispatch("server", "open", new Event("open", { cancelable: true }));
      }
      case "server-message": {
        const conn = this.connections.get(event.connectionId);
        if (!conn) return false;
        return conn.dispatch("server", "message", messageEvent("message", event.data));
      }
      case "server-error": {
        const conn = this.connections.get(event.connectionId);
        if (!conn) return false;
        return conn.dispatch("server", "error", new Event("error", { cancelable: true }));
      }
      case "server-close": {
        const conn = this.connections.get(event.connectionId);
        if (!conn) return false;
        return conn.dispatch(
          "server",
          "close",
          closeEvent("close", event.code ?? 1000, event.reason ?? "")
        );
      }
      default:
        return false;
    }
  }
}

export interface WebSocketLink {
  /** All clients connected through this link. */
  clients: Set<WebSocketClientConnectionProtocol>;
  addEventListener(
    event: "connection",
    listener: WebSocketConnectionListener
  ): WebSocketHandler;
  /** Send to every connected client. */
  broadcast(data: WebSocketData): void;
  /** Send to every connected client except the given one(s). */
  broadcastExcept(
    clients:
      | WebSocketClientConnectionProtocol
      | WebSocketClientConnectionProtocol[],
    data: WebSocketData
  ): void;
}

function createLink(url: string | RegExp): WebSocketLink {
  const clients = new Set<WebSocketClientConnectionProtocol>();

  return {
    clients,
    addEventListener(event, listener) {
      if (event !== "connection") {
        throw new TypeError("ws links only emit the 'connection' event");
      }
      const handler = new WebSocketHandler(url);
      handler.on((connection) => {
        clients.add(connection.client);
        connection.client.addEventListener("close", () => {
          clients.delete(connection.client);
        });
      });
      handler.on(listener);
      registerCollected(handler);
      return handler;
    },
    broadcast(data) {
      for (const client of clients) {
        client.send(data);
      }
    },
    broadcastExcept(exceptClients, data) {
      const excluded = new Set(
        (Array.isArray(exceptClients) ? exceptClients : [exceptClients]).map(
          (client) => client.id
        )
      );
      for (const client of clients) {
        if (!excluded.has(client.id)) {
          client.send(data);
        }
      }
    },
  };
}

export const ws = {
  link: createLink,
};

export function isWsHandler(value: unknown): value is WebSocketHandler {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as WebSocketHandler).kind === "websocket"
  );
}
