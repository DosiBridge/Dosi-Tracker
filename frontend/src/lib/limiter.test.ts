import { describe, expect, it } from "vitest";
import { createLimiter } from "./limiter";

/** A task whose completion the test controls. */
function deferred<T = void>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("createLimiter", () => {
  it("never runs more than the cap at once, and starts queued tasks in FIFO order", async () => {
    const limiter = createLimiter(2);
    const gates = [deferred(), deferred(), deferred(), deferred()];
    const started: number[] = [];
    const runs = gates.map((g, i) =>
      limiter.run(() => {
        started.push(i);
        return g.promise;
      }),
    );

    expect(started).toEqual([0, 1]);
    expect(limiter.active).toBe(2);
    expect(limiter.pending).toBe(2);

    gates[1].resolve();
    await tick();
    expect(started).toEqual([0, 1, 2]);
    expect(limiter.active).toBe(2);

    gates[0].resolve();
    gates[2].resolve();
    gates[3].resolve();
    await Promise.all(runs);
    expect(started).toEqual([0, 1, 2, 3]);
    expect(limiter.active).toBe(0);
    expect(limiter.pending).toBe(0);
  });

  it("frees the slot when a task fails or throws synchronously", async () => {
    const limiter = createLimiter(1);
    await expect(limiter.run(() => Promise.reject(new Error("boom")))).rejects.toThrow("boom");
    await expect(
      limiter.run(() => {
        throw new Error("sync");
      }),
    ).rejects.toThrow("sync");
    await expect(limiter.run(async () => 42)).resolves.toBe(42);
    expect(limiter.active).toBe(0);
  });

  it("drops a queued task whose signal aborts, without ever starting it", async () => {
    const limiter = createLimiter(1);
    const gate = deferred();
    const first = limiter.run(() => gate.promise);
    const controller = new AbortController();
    let started = false;
    const second = limiter.run(async () => {
      started = true;
    }, controller.signal);

    expect(limiter.pending).toBe(1);
    controller.abort();
    await expect(second).rejects.toMatchObject({ name: "AbortError" });
    expect(limiter.pending).toBe(0);

    gate.resolve();
    await first;
    expect(started).toBe(false);
  });

  it("rejects immediately when the signal is already aborted", async () => {
    const limiter = createLimiter(3);
    const controller = new AbortController();
    controller.abort();
    await expect(limiter.run(async () => 1, controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(limiter.active).toBe(0);
  });

  it("treats a nonsense cap as 1", async () => {
    const limiter = createLimiter(0);
    const gate = deferred();
    void limiter.run(() => gate.promise);
    void limiter.run(async () => undefined);
    expect(limiter.active).toBe(1);
    expect(limiter.pending).toBe(1);
    gate.resolve();
  });
});
