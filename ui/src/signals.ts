export type Subscriber<T> = (value: T) => void;

export interface ReadonlySignal<T> {
  readonly value: T;
  subscribe(subscriber: Subscriber<T>): () => void;
}

export interface Signal<T> extends ReadonlySignal<T> {
  set(value: T): void;
}

export function signal<T>(initial: T): Signal<T> {
  let current = initial;
  const subscribers = new Set<Subscriber<T>>();

  return {
    get value(): T {
      return current;
    },
    set(value: T): void {
      if (Object.is(value, current)) {
        return;
      }
      current = value;
      for (const subscriber of [...subscribers]) {
        subscriber(current);
      }
    },
    subscribe(subscriber: Subscriber<T>): () => void {
      subscribers.add(subscriber);
      subscriber(current);
      return () => {
        subscribers.delete(subscriber);
      };
    },
  };
}

export function computed<T>(derive: () => T, ...sources: ReadonlySignal<unknown>[]): ReadonlySignal<T> {
  const derived = signal(derive());
  for (const source of sources) {
    source.subscribe(() => derived.set(derive()));
  }
  return derived;
}
