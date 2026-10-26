import { describe, expect, it } from "vitest";
import {
  AGENT_PLATFORMS,
  AGENT_PLATFORM_ORDER,
  LINUX_COMMANDS,
  MACOS_COMMANDS,
  RELEASE_ASSET_FILES,
  SERVER_PLACEHOLDER,
  WINDOWS_COMMANDS,
  checksumsUrl,
  releaseAssetUrl,
  releasesPageUrl,
} from "./agent-downloads";
import { DEFAULT_DOWNLOAD_BASE_URL } from "./runtime-config";

describe("release assets", () => {
  it("are exactly the names the release pipeline publishes", () => {
    expect(RELEASE_ASSET_FILES).toEqual({
      windowsInstaller: "dosi-tracker-setup.exe",
      macosZip: "DosiTracker-macos.zip",
      linuxTarball: "dosi-tracker-linux-x86_64.tar.gz",
      linuxDeb: "dosi-tracker_amd64.deb",
      linuxRpm: "dosi-tracker.x86_64.rpm",
      checksums: "SHA256SUMS",
    });
  });

  it("every platform offers at least one asset, each a published name, and every published installer is offered", () => {
    const offered = new Set<string>();
    for (const platform of AGENT_PLATFORM_ORDER) {
      const info = AGENT_PLATFORMS[platform];
      expect(info.platform).toBe(platform);
      expect(info.assets.length).toBeGreaterThan(0);
      expect(info.guidePath).toBe(`/install/${platform}`);
      for (const asset of info.assets) offered.add(asset.file);
    }
    const installers = Object.values(RELEASE_ASSET_FILES).filter((f) => f !== RELEASE_ASSET_FILES.checksums);
    expect([...offered].sort()).toEqual([...installers].sort());
  });

  it("lists Windows, macOS, then Linux", () => {
    expect(AGENT_PLATFORM_ORDER).toEqual(["windows", "macos", "linux"]);
  });
});

describe("download URLs", () => {
  it("join the base and the asset name exactly once, encoding the name", () => {
    expect(releaseAssetUrl(DEFAULT_DOWNLOAD_BASE_URL, "dosi-tracker-setup.exe")).toBe(
      "https://github.com/DosiBridge/Dosi-Tracker/releases/latest/download/dosi-tracker-setup.exe",
    );
    expect(releaseAssetUrl("https://files.example.com/agents/", "a b.zip")).toBe("https://files.example.com/agents/a%20b.zip");
    expect(checksumsUrl("https://files.example.com/agents")).toBe("https://files.example.com/agents/SHA256SUMS");
  });

  it("links the human releases page only for GitHub Releases", () => {
    expect(releasesPageUrl(DEFAULT_DOWNLOAD_BASE_URL)).toBe("https://github.com/DosiBridge/Dosi-Tracker/releases");
    expect(releasesPageUrl("https://github.com/acme/tracker-fork/releases/latest/download/")).toBe("https://github.com/acme/tracker-fork/releases");
    expect(releasesPageUrl("https://files.example.com/agents")).toBeNull();
    expect(releasesPageUrl("https://github.com/acme/tracker/releases/download/v1.2.0")).toBeNull();
  });
});

describe("commands", () => {
  const server = "https://tracker-api.example.com";

  it("carry this site's server address, or a visible placeholder when it has none", () => {
    expect(WINDOWS_COMMANDS.silentInstall(server)).toBe("dosi-tracker-setup.exe /S /SERVER=https://tracker-api.example.com");
    expect(WINDOWS_COMMANDS.silentInstall(null)).toBe(`dosi-tracker-setup.exe /S /SERVER=${SERVER_PLACEHOLDER}`);
    expect(MACOS_COMMANDS.setServer(server)).toBe('defaults write com.dosibridge.tracker DosiApiBaseURL "https://tracker-api.example.com"');
    expect(LINUX_COMMANDS.login(server)).toBe("dosi-tracker login --server https://tracker-api.example.com");
    expect(LINUX_COMMANDS.installTarball(null).at(-1)).toBe(`./install.sh --server ${SERVER_PLACEHOLDER}`);
  });

  it("install the published Linux files by name", () => {
    expect(LINUX_COMMANDS.installDeb).toBe("sudo apt install ./dosi-tracker_amd64.deb");
    expect(LINUX_COMMANDS.installRpm).toBe("sudo dnf install ./dosi-tracker.x86_64.rpm");
    expect(LINUX_COMMANDS.installTarball(server)[0]).toBe("tar -xzf dosi-tracker-linux-x86_64.tar.gz");
    expect(LINUX_COMMANDS.verify).toBe("sha256sum -c --ignore-missing SHA256SUMS");
  });

  it("never put a password on the command line", () => {
    const all = [
      ...Object.values(LINUX_COMMANDS).flatMap((c) => (typeof c === "function" ? c(server) : c)),
      ...Object.values(WINDOWS_COMMANDS).flatMap((c) => (typeof c === "function" ? c(server) : c)),
      ...Object.values(MACOS_COMMANDS).flatMap((c) => (typeof c === "function" ? c(server) : c)),
    ];
    for (const command of all) expect(command).not.toMatch(/--password(?!-stdin|-file)|\s-p\s/);
  });

  it("build the macOS app where the build script really puts it", () => {
    expect(MACOS_COMMANDS.build).toEqual(["cd clients/macos", "bash scripts/build-app.sh", 'cp -R ".build/dist/Dosi Tracker.app" /Applications/']);
  });
});
