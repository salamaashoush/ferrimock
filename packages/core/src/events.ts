/**
 * Typed event emitter for MSW-compatible lifecycle events.
 */

export interface LifecycleEventMap {
  "request:start": { request: Request; requestId: string };
  "request:match": { request: Request; requestId: string };
  "request:unhandled": { request: Request; requestId: string };
  "request:end": { request: Request; requestId: string };
  "response:mocked": { request: Request; requestId: string; response: Response };
  "response:bypass": { request: Request; requestId: string; response: Response };
  unhandledException: { request: Request; requestId: string; error: Error };
}

type EventListener<T> = (data: T) => void;

export class LifecycleEvents {
  private listeners = new Map<string, Set<EventListener<any>>>();

  on<K extends keyof LifecycleEventMap>(
    event: K,
    listener: EventListener<LifecycleEventMap[K]>
  ): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(listener);
  }

  removeListener<K extends keyof LifecycleEventMap>(
    event: K,
    listener: EventListener<LifecycleEventMap[K]>
  ): void {
    this.listeners.get(event)?.delete(listener);
  }

  removeAllListeners(event?: keyof LifecycleEventMap): void {
    if (event) {
      this.listeners.delete(event);
    } else {
      this.listeners.clear();
    }
  }

  emit<K extends keyof LifecycleEventMap>(
    event: K,
    data: LifecycleEventMap[K]
  ): void {
    const set = this.listeners.get(event);
    if (set) {
      for (const listener of set) {
        try {
          listener(data);
        } catch {
          // lifecycle listeners should not throw
        }
      }
    }
  }
}
