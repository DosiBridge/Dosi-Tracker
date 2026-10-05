// useReportingZone: the zone every report's days belong to — the viewer's own
// preference, else the workspace default, else the browser's zone. A demo
// session never touches the network; a live one says when it is `ready`, falls
// through settings an older server lacks (or a member may not read), and can be
// refreshed after Settings saves a new zone.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useReportingZone } from "./use-reporting-zone";
import { browserTimeZone } from "@/lib/time-zone";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";

function res(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

/** What the backend answers for the two settings reads (a Response, or a status for a failure). */
let answers: { preferences: unknown | number; settings: unknown | number };
let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  localStorage.clear();
  answers = { preferences: { timeZone: null }, settings: { defaultTimeZone: null } };
  fetchMock = vi.fn<typeof fetch>(async (url) => {
    const path = String(url).replace(API_BASE, "");
    const answer =
      path === "/api/app/workspace/my-preferences" ? answers.preferences : path === "/api/app/workspace/settings" ? answers.settings : 404;
    return typeof answer === "number" ? res({ error: { message: "nope" } }, answer) : res(answer);
  });
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

function Probe({ live }: { live: boolean }) {
  const zone = useReportingZone(live);
  return (
    <div>
      <span data-testid="zone">{zone.timeZone}</span>
      <span data-testid="source">{zone.source}</span>
      <span data-testid="ready">{String(zone.ready)}</span>
      <button type="button" onClick={zone.refresh}>
        Refresh
      </button>
    </div>
  );
}

const shown = (id: string) => screen.getByTestId(id).textContent;

describe("useReportingZone", () => {
  it("demo: reports in the browser's zone at once and never touches the network", () => {
    render(<Probe live={false} />);
    expect(shown("zone")).toBe(browserTimeZone());
    expect(shown("source")).toBe("browser");
    expect(shown("ready")).toBe("true");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("live: the viewer's own zone wins over the workspace default", async () => {
    answers = { preferences: { timeZone: "Pacific/Chatham" }, settings: { defaultTimeZone: "Atlantic/Azores" } };
    render(<Probe live />);
    // Until both answers are in, the browser's zone stands in and the hook isn't ready.
    expect(shown("ready")).toBe("false");
    expect(shown("source")).toBe("browser");
    await waitFor(() => expect(shown("ready")).toBe("true"));
    expect(shown("zone")).toBe("Pacific/Chatham");
    expect(shown("source")).toBe("user");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("live: falls back to the workspace default when the viewer chose none (or an unusable one)", async () => {
    answers = { preferences: { timeZone: "Mars/Phobos" }, settings: { defaultTimeZone: "atlantic/azores" } };
    render(<Probe live />);
    await waitFor(() => expect(shown("ready")).toBe("true"));
    expect(shown("zone")).toBe("Atlantic/Azores");
    expect(shown("source")).toBe("workspace");
  });

  it("live: an older server (404) and a member who may not read workspace settings (403) get the browser's zone", async () => {
    answers = { preferences: 404, settings: 403 };
    render(<Probe live />);
    await waitFor(() => expect(shown("ready")).toBe("true"));
    expect(shown("zone")).toBe(browserTimeZone());
    expect(shown("source")).toBe("browser");
  });

  it("refresh() reads the settings again (e.g. after Settings saved a new zone)", async () => {
    const user = userEvent.setup();
    answers = { preferences: { timeZone: "Pacific/Chatham" }, settings: { defaultTimeZone: null } };
    render(<Probe live />);
    await waitFor(() => expect(shown("zone")).toBe("Pacific/Chatham"));

    answers = { preferences: { timeZone: "Asia/Kathmandu" }, settings: { defaultTimeZone: null } };
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(shown("zone")).toBe("Asia/Kathmandu"));
    expect(fetchMock).toHaveBeenCalledTimes(4);
  });

  it("a session that goes live again never reuses the previous session's answers", async () => {
    answers = { preferences: { timeZone: "Pacific/Chatham" }, settings: { defaultTimeZone: null } };
    const { rerender } = render(<Probe live />);
    await waitFor(() => expect(shown("zone")).toBe("Pacific/Chatham"));

    rerender(<Probe live={false} />);
    expect(shown("source")).toBe("browser");
    expect(shown("ready")).toBe("true");

    answers = { preferences: { timeZone: null }, settings: { defaultTimeZone: "Atlantic/Azores" } };
    rerender(<Probe live />);
    expect(shown("ready")).toBe("false");
    expect(shown("zone")).not.toBe("Pacific/Chatham");
    await waitFor(() => expect(shown("zone")).toBe("Atlantic/Azores"));
    expect(shown("source")).toBe("workspace");
  });

  it("ignores answers that arrive after unmounting", async () => {
    let release: (r: Response) => void = () => {};
    fetchMock.mockImplementation(() => new Promise<Response>((resolve) => (release = resolve)));
    const { unmount } = render(<Probe live />);
    unmount();
    await act(async () => {
      release(res({ timeZone: "Pacific/Chatham" }));
    });
    expect(fetchMock).toHaveBeenCalled();
  });
});
