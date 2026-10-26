// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { GET } from "./route";

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("GET /healthz", () => {
  it("answers 200 and never caches when the deployment can serve sign-ins", async () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("DOSI_API_URL", "https://api.example.com");
    vi.stubEnv("DOSI_DEMO_MODE", "");
    const res = GET();
    expect(res.status).toBe(200);
    expect(res.headers.get("cache-control")).toBe("no-store");
    expect(await res.json()).toEqual({ status: "ok", problems: [], demoMode: false });
  });

  it("answers 503 with the reason when production has no API address", async () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("DOSI_API_URL", "");
    vi.stubEnv("NEXT_PUBLIC_API_URL", "");
    vi.stubEnv("DOSI_DEMO_MODE", "");
    const res = GET();
    expect(res.status).toBe(503);
    const body = await res.json();
    expect(body.status).toBe("misconfigured");
    expect(body.problems[0]).toMatch(/DOSI_API_URL is not set/);
  });

  it("reports demo mode, so a demo deployment is never mistaken for a live one", async () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("DOSI_DEMO_MODE", "1");
    const res = GET();
    expect(res.status).toBe(200);
    expect((await res.json()).demoMode).toBe(true);
  });

  it("stays healthy, but says so, when only the download location is wrong", async () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("DOSI_API_URL", "https://api.example.com");
    vi.stubEnv("DOSI_DOWNLOAD_BASE_URL", "not-a-url");
    const res = GET();
    expect(res.status).toBe(200);
    expect((await res.json()).problems).toEqual([expect.stringMatching(/DOSI_DOWNLOAD_BASE_URL/)]);
  });
});
