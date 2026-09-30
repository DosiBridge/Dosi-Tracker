/* ============================================================================
 * CONCURRENCY LIMITER — at most N async tasks in flight, the rest queued FIFO.
 *
 * A page of activity cards can ask for dozens of screenshot thumbnails at once;
 * firing them all together starves the requests the user is actually waiting
 * on. Tasks wait for a free slot, and a task whose signal aborts while it is
 * still queued never starts (its card scrolled away or unmounted).
 * ========================================================================== */

export interface Limiter {
  /** Run `task` when a slot is free. Rejects with an AbortError if `signal` aborts before it starts. */
  run<T>(task: () => Promise<T>, signal?: AbortSignal): Promise<T>;
  /** Tasks currently running. */
  readonly active: number;
  /** Tasks waiting for a slot. */
  readonly pending: number;
}

function abortError(): Error {
  const err = new Error("Aborted before it started");
  err.name = "AbortError";
  return err;
}

export function createLimiter(maxConcurrent: number): Limiter {
  const max = Math.max(1, Math.floor(maxConcurrent) || 1);
  let active = 0;
  const queue: Array<() => void> = [];

  const next = () => {
    while (active < max && queue.length > 0) queue.shift()!();
  };

  return {
    run<T>(task: () => Promise<T>, signal?: AbortSignal): Promise<T> {
      return new Promise<T>((resolve, reject) => {
        if (signal?.aborted) {
          reject(abortError());
          return;
        }
        const start = () => {
          signal?.removeEventListener("abort", onAbort);
          active++;
          let settled: Promise<T>;
          try {
            settled = task();
          } catch (err) {
            settled = Promise.reject(err);
          }
          settled.then(resolve, reject).finally(() => {
            active--;
            next();
          });
        };
        const onAbort = () => {
          const at = queue.indexOf(start);
          if (at !== -1) queue.splice(at, 1);
          reject(abortError());
        };
        signal?.addEventListener("abort", onAbort, { once: true });
        queue.push(start);
        next();
      });
    },
    get active() {
      return active;
    },
    get pending() {
      return queue.length;
    },
  };
}
