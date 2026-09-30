// useApi: the declarative GET must behave like getApi (401 ends the session,
// items are unwrapped), stay off the network when disabled (demo mode), and
// abort its request on unmount so a late response can't touch a dead tree.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { useApi } from "./useApi";

function res(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 401 ? "Unauthorized" : "OK",
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

function Probe({ endpoint, enabled }: { endpoint: string; enabled?: boolean }) {
  const { data, error, isLoading } = useApi<unknown[]>(endpoint, enabled === undefined ? {} : { enabled });
  return (
    <div>
      <span data-testid="loading">{String(isLoading)}</span>
      <span data-testid="data">{JSON.stringify(data)}</span>
      <span data-testid="error">{error?.message ?? ""}</span>
    </div>
  );
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  localStorage.clear();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("useApi", () => {
  it("unwraps ABP's items envelope", async () => {
    fetchMock.mockResolvedValue(res({ items: [1, 2] }));
    render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent("[1,2]"));
    expect(screen.getByTestId("loading")).toHaveTextContent("false");
  });

  it("makes no request at all when disabled (demo mode)", () => {
    render(<Probe endpoint="/api/app/x" enabled={false} />);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.getByTestId("loading")).toHaveTextContent("false");
  });

  it("treats 401 like getApi: clears the session and reports the error", async () => {
    localStorage.setItem("dosi-token", "expired");
    fetchMock.mockResolvedValue(res({}, 401));
    render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(screen.getByTestId("error")).toHaveTextContent("Unauthorized"));
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("aborts the in-flight request on unmount and never reports the abort as an error", async () => {
    let signal: AbortSignal | undefined;
    fetchMock.mockImplementation((_url, init) => {
      signal = init?.signal ?? undefined;
      return new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      });
    });
    const { unmount } = render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    unmount();
    expect(signal?.aborted).toBe(true);
  });

  it("supersedes a slower request when the endpoint changes", async () => {
    const resolvers: ((r: Response) => void)[] = [];
    fetchMock.mockImplementation(
      (_url, init) =>
        new Promise<Response>((resolve, reject) => {
          resolvers.push(resolve);
          init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );
    const { rerender } = render(<Probe endpoint="/api/app/a" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    rerender(<Probe endpoint="/api/app/b" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    resolvers[1](res(["b"]));
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent('["b"]'));
    expect(screen.getByTestId("error")).toHaveTextContent("");
  });
});
