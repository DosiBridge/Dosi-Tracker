import { afterEach, describe, expect, it } from "vitest";
import {
  ApiNotConfiguredError,
  DEFAULT_DOWNLOAD_BASE_URL,
  DEV_API_BASE_URL,
  RUNTIME_CONFIG_ELEMENT_ID,
  apiOrigin,
  apiUrl,
  envFlag,
  isLoopbackHost,
  normalizeBaseUrl,
  requireApiBaseUrl,
  resetRuntimeConfigCache,
  resolveRuntimeConfig,
  runtimeConfig,
  serializeRuntimeConfig,
  type RuntimeConfig,
} from "./runtime-config";

function renderConfigBlock(config: unknown) {
  const el = document.createElement("script");
  el.type = "application/json";
  el.id = RUNTIME_CONFIG_ELEMENT_ID;
  el.textContent = typeof config === "string" ? config : JSON.stringify(config);
  document.head.appendChild(el);
}

afterEach(() => {
  document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.remove();
  resetRuntimeConfigCache();
});

describe("envFlag", () => {
  it("reads common spellings of on, everything else is off", () => {
    for (const on of ["1", "true", "TRUE", " yes ", "on"]) expect(envFlag(on)).toBe(true);
    for (const off of [undefined, "", "0", "false", "off", "no", "enabled"]) expect(envFlag(off)).toBe(false);
  });
});

describe("isLoopbackHost", () => {
  it("knows this machine's names", () => {
    for (const host of ["localhost", "LOCALHOST", "127.0.0.1", "127.10.0.3", "[::1]", "api.localhost"]) {
      expect(isLoopbackHost(host)).toBe(true);
    }
    for (const host of ["example.com", "10.0.0.1", "localhost.example.com", "128.0.0.1"]) {
      expect(isLoopbackHost(host)).toBe(false);
    }
  });
});

describe("normalizeBaseUrl", () => {
  it("strips trailing slashes so endpoint paths never double up", () => {
    expect(normalizeBaseUrl("https://api.example.com/", "X").url).toBe("https://api.example.com");
    expect(normalizeBaseUrl("https://api.example.com///", "X").url).toBe("https://api.example.com");
    expect(normalizeBaseUrl("  https://example.com/tracker/  ", "X").url).toBe("https://example.com/tracker");
  });

  it("keeps a port and lowercases the host", () => {
    expect(normalizeBaseUrl("https://API.Example.com:8443", "X").url).toBe("https://api.example.com:8443");
  });

  it("refuses what isn't a plain http(s) base URL, naming the variable", () => {
    expect(normalizeBaseUrl("not a url", "DOSI_API_URL").problem).toMatch(/DOSI_API_URL is not a valid URL/);
    expect(normalizeBaseUrl("ftp://example.com", "DOSI_API_URL").problem).toMatch(/must be an http\(s\) URL/);
    expect(normalizeBaseUrl("https://user:pw@example.com", "DOSI_API_URL").problem).toMatch(/user name or other credentials/);
    expect(normalizeBaseUrl("https://example.com/?a=1", "DOSI_API_URL").problem).toMatch(/query string or fragment/);
    expect(normalizeBaseUrl("https://example.com/#x", "DOSI_API_URL").problem).toMatch(/query string or fragment/);
  });

  it("accepts plain http only for loopback hosts or with the explicit opt-in", () => {
    expect(normalizeBaseUrl("http://localhost:6006", "X").url).toBe("http://localhost:6006");
    expect(normalizeBaseUrl("http://127.0.0.1:6006/", "X").url).toBe("http://127.0.0.1:6006");
    const refused = normalizeBaseUrl("http://tracker.internal", "DOSI_API_URL");
    expect(refused.url).toBeNull();
    expect(refused.problem).toMatch(/DOSI_ALLOW_INSECURE_HTTP=true/);
    expect(normalizeBaseUrl("http://tracker.internal", "X", { allowInsecureHttp: true }).url).toBe("http://tracker.internal");
  });
});

describe("resolveRuntimeConfig", () => {
  it("uses DOSI_API_URL, normalised", () => {
    const config = resolveRuntimeConfig({ NODE_ENV: "production", DOSI_API_URL: "https://api.example.com/" });
    expect(config).toEqual({
      apiBaseUrl: "https://api.example.com",
      downloadBaseUrl: DEFAULT_DOWNLOAD_BASE_URL,
      demoMode: false,
      problems: [],
    });
  });

  it("still honours the legacy NEXT_PUBLIC_API_URL, but DOSI_API_URL wins", () => {
    expect(resolveRuntimeConfig({ NODE_ENV: "production", NEXT_PUBLIC_API_URL: "https://legacy.example.com" }).apiBaseUrl).toBe(
      "https://legacy.example.com",
    );
    expect(
      resolveRuntimeConfig({ NODE_ENV: "production", NEXT_PUBLIC_API_URL: "https://legacy.example.com", DOSI_API_URL: "https://new.example.com" })
        .apiBaseUrl,
    ).toBe("https://new.example.com");
  });

  it("never falls back to localhost in production: no API, and an operator-facing problem", () => {
    const config = resolveRuntimeConfig({ NODE_ENV: "production" });
    expect(config.apiBaseUrl).toBeNull();
    expect(config.problems).toEqual([expect.stringMatching(/DOSI_API_URL is not set/)]);
    expect(resolveRuntimeConfig({ NODE_ENV: "production", DOSI_API_URL: "   " }).apiBaseUrl).toBeNull();
  });

  it("falls back to the dev backend outside production, and for a demo (which never calls the API)", () => {
    expect(resolveRuntimeConfig({ NODE_ENV: "development" }).apiBaseUrl).toBe(DEV_API_BASE_URL);
    expect(resolveRuntimeConfig({ NODE_ENV: "test" }).apiBaseUrl).toBe(DEV_API_BASE_URL);
    expect(resolveRuntimeConfig({}).apiBaseUrl).toBe(DEV_API_BASE_URL);
    const demo = resolveRuntimeConfig({ NODE_ENV: "production", DOSI_DEMO_MODE: "1" });
    expect(demo.apiBaseUrl).toBe(DEV_API_BASE_URL);
    expect(demo.demoMode).toBe(true);
    expect(demo.problems).toEqual([]);
  });

  it("reports an invalid API URL instead of using it", () => {
    const config = resolveRuntimeConfig({ NODE_ENV: "production", DOSI_API_URL: "http://api.example.com" });
    expect(config.apiBaseUrl).toBeNull();
    expect(config.problems[0]).toMatch(/DOSI_API_URL uses plain http/);
    const legacy = resolveRuntimeConfig({ NODE_ENV: "production", NEXT_PUBLIC_API_URL: "nope" });
    expect(legacy.problems[0]).toMatch(/NEXT_PUBLIC_API_URL is not a valid URL/);
  });

  it("accepts insecure http for the API and downloads only with the explicit opt-in", () => {
    const config = resolveRuntimeConfig({
      NODE_ENV: "production",
      DOSI_API_URL: "http://api.lan",
      DOSI_DOWNLOAD_BASE_URL: "http://files.lan/agents/",
      DOSI_ALLOW_INSECURE_HTTP: "true",
    });
    expect(config.apiBaseUrl).toBe("http://api.lan");
    expect(config.downloadBaseUrl).toBe("http://files.lan/agents");
    expect(config.problems).toEqual([]);
  });

  it("uses a configured download base URL, or keeps the default and says why", () => {
    expect(
      resolveRuntimeConfig({ DOSI_DOWNLOAD_BASE_URL: "https://downloads.example.com/dosi/v1.2.0/" }).downloadBaseUrl,
    ).toBe("https://downloads.example.com/dosi/v1.2.0");
    const bad = resolveRuntimeConfig({ DOSI_DOWNLOAD_BASE_URL: "nope" });
    expect(bad.downloadBaseUrl).toBe(DEFAULT_DOWNLOAD_BASE_URL);
    expect(bad.problems[0]).toMatch(/DOSI_DOWNLOAD_BASE_URL is not a valid URL.*default download location/);
  });

  it("demo mode is off unless explicitly switched on", () => {
    expect(resolveRuntimeConfig({ NODE_ENV: "production" }).demoMode).toBe(false);
    expect(resolveRuntimeConfig({ NODE_ENV: "development" }).demoMode).toBe(false);
    expect(resolveRuntimeConfig({ DOSI_DEMO_MODE: "0" }).demoMode).toBe(false);
    expect(resolveRuntimeConfig({ DOSI_DEMO_MODE: "true" }).demoMode).toBe(true);
  });
});

describe("runtimeConfig in the browser", () => {
  const served: RuntimeConfig = {
    apiBaseUrl: "https://api.served.example",
    downloadBaseUrl: "https://dl.served.example",
    demoMode: false,
    problems: [],
  };

  it("reads what the server rendered into the page, and keeps it", () => {
    renderConfigBlock(served);
    expect(runtimeConfig()).toEqual(served);
    document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.remove();
    expect(runtimeConfig()).toEqual(served);
  });

  it("falls back to the environment when the page has no (valid) data block", () => {
    expect(runtimeConfig().apiBaseUrl).toBe(DEV_API_BASE_URL);
    renderConfigBlock("{not json");
    expect(runtimeConfig().apiBaseUrl).toBe(DEV_API_BASE_URL);
    document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.remove();
    renderConfigBlock({ apiBaseUrl: 42 });
    expect(runtimeConfig().apiBaseUrl).toBe(DEV_API_BASE_URL);
    document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.remove();
    renderConfigBlock("");
    expect(runtimeConfig().apiBaseUrl).toBe(DEV_API_BASE_URL);
  });

  it("builds endpoint URLs from the configured base", () => {
    renderConfigBlock(served);
    expect(apiUrl("/connect/token")).toBe("https://api.served.example/connect/token");
    expect(apiUrl("api/app/workspaces/mine")).toBe("https://api.served.example/api/app/workspaces/mine");
    expect(apiOrigin()).toBe("https://api.served.example");
  });

  it("refuses to build URLs when no API is configured", () => {
    renderConfigBlock({ ...served, apiBaseUrl: null });
    expect(() => requireApiBaseUrl()).toThrow(ApiNotConfiguredError);
    expect(() => apiUrl("/api/x")).toThrow(/no Dosi-Tracker API server configured/);
    expect(apiOrigin()).toBeNull();
  });

  it("the origin drops any base path", () => {
    expect(apiOrigin({ ...served, apiBaseUrl: "https://example.com:8443/tracker" })).toBe("https://example.com:8443");
  });
});

describe("serializeRuntimeConfig", () => {
  it("can't be broken out of its <script> element", () => {
    const json = serializeRuntimeConfig({
      apiBaseUrl: null,
      downloadBaseUrl: "https://x.example",
      demoMode: false,
      problems: ["</script><script>alert(1)</script>"],
    });
    expect(json).not.toContain("<");
    expect(JSON.parse(json).problems[0]).toBe("</script><script>alert(1)</script>");
  });
});
