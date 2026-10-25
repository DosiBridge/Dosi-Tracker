/* ============================================================================
 * DESKTOP AGENTS — the one source of truth for where each agent comes from and
 * how it is installed. The dashboard's Download page, the public /install
 * guides and the sign-in details all read from here, so a release asset name
 * or a command changes in one place.
 *
 * Release assets are published on every release under these version-less
 * names, so `<download base>/<file>` always serves the newest build. The base
 * is runtime configuration (DOSI_DOWNLOAD_BASE_URL, default GitHub Releases
 * "latest/download"), so a self-hosted deployment can point it at its own
 * file server.
 * ========================================================================== */

export type AgentPlatform = "windows" | "macos" | "linux";

/** The release asset names (the release pipeline uploads exactly these). */
export const RELEASE_ASSET_FILES = {
  windowsInstaller: "dosi-tracker-setup.exe",
  macosZip: "DosiTracker-macos.zip",
  linuxTarball: "dosi-tracker-linux-x86_64.tar.gz",
  linuxDeb: "dosi-tracker_amd64.deb",
  linuxRpm: "dosi-tracker.x86_64.rpm",
  checksums: "SHA256SUMS",
} as const;

export type ReleaseAssetFile = (typeof RELEASE_ASSET_FILES)[keyof typeof RELEASE_ASSET_FILES];

export interface ReleaseAsset {
  file: ReleaseAssetFile;
  /** What it is, e.g. "Windows installer". */
  label: string;
  /** Who it is for, e.g. "Ubuntu 22.04+ and Debian 12+". */
  note: string;
}

export interface AgentPlatformInfo {
  platform: AgentPlatform;
  name: string;
  /** Supported systems, in one line. */
  requirement: string;
  /** The public setup guide. */
  guidePath: string;
  /** Downloads, the recommended one first. */
  assets: readonly ReleaseAsset[];
  /** Where the agent shows itself while it runs. */
  home: string;
}

/** The public guides index (no sign-in needed). */
export const INSTALL_GUIDES_PATH = "/install";

export const AGENT_PLATFORMS: Readonly<Record<AgentPlatform, AgentPlatformInfo>> = {
  windows: {
    platform: "windows",
    name: "Windows",
    requirement: "Windows 10 (1809 or newer) or Windows 11, 64-bit (x86_64)",
    guidePath: "/install/windows",
    assets: [
      {
        file: RELEASE_ASSET_FILES.windowsInstaller,
        label: "Windows installer",
        note: "Installs for your Windows user only; no administrator rights",
      },
    ],
    home: "the notification area (system tray)",
  },
  macos: {
    platform: "macos",
    name: "macOS",
    requirement: "macOS 14 Sonoma or later, Apple silicon or Intel",
    guidePath: "/install/macos",
    assets: [
      {
        file: RELEASE_ASSET_FILES.macosZip,
        label: "macOS app (zip)",
        note: "Universal app for Apple silicon and Intel",
      },
    ],
    home: "the menu bar",
  },
  linux: {
    platform: "linux",
    name: "Linux",
    requirement: "x86_64 with glibc 2.35+ (Ubuntu 22.04+, Debian 12+, Fedora 43+, Arch); X11 or Wayland",
    guidePath: "/install/linux",
    assets: [
      { file: RELEASE_ASSET_FILES.linuxDeb, label: "Debian package (.deb)", note: "Ubuntu 22.04+ and Debian 12+" },
      { file: RELEASE_ASSET_FILES.linuxRpm, label: "RPM package (.rpm)", note: "Fedora 43+" },
      {
        file: RELEASE_ASSET_FILES.linuxTarball,
        label: "Tarball (.tar.gz)",
        note: "Any distribution; installs to ~/.local without root",
      },
    ],
    home: "the system tray where your desktop has one, and the dosi-tracker command",
  },
};

/** The platforms in the order the pages list them. */
export const AGENT_PLATFORM_ORDER: readonly AgentPlatform[] = ["windows", "macos", "linux"];

/** Where one release asset downloads from. */
export function releaseAssetUrl(downloadBaseUrl: string, file: string): string {
  return `${downloadBaseUrl.replace(/\/+$/, "")}/${encodeURIComponent(file)}`;
}

/** Where the published SHA-256 checksums of every asset download from. */
export function checksumsUrl(downloadBaseUrl: string): string {
  return releaseAssetUrl(downloadBaseUrl, RELEASE_ASSET_FILES.checksums);
}

const GITHUB_LATEST = /^https:\/\/github\.com\/([^/]+)\/([^/]+)\/releases\/latest\/download$/i;

/**
 * The human releases page (release notes, older versions) when the downloads
 * come from GitHub Releases; null for a self-hosted file server.
 */
export function releasesPageUrl(downloadBaseUrl: string): string | null {
  const match = GITHUB_LATEST.exec(downloadBaseUrl.replace(/\/+$/, ""));
  return match ? `https://github.com/${match[1]}/${match[2]}/releases` : null;
}

/** Shown in a command where this site's API address belongs, when it has none configured. */
export const SERVER_PLACEHOLDER = "<server address>";

/* ----------------------------- Commands ----------------------------- */
// Every command a guide shows, built here so the pages, tests and docs agree.

/** The address agents sign in to, or the placeholder. */
const serverOr = (server: string | null) => server ?? SERVER_PLACEHOLDER;

export const WINDOWS_COMMANDS = {
  /** Silent per-user install that also sets the server address (managed deployment). */
  silentInstall: (server: string | null) => `${RELEASE_ASSET_FILES.windowsInstaller} /S /SERVER=${serverOr(server)}`,
  /** Quit gracefully: the block in progress is saved and uploaded. */
  quit: String.raw`"%LOCALAPPDATA%\Programs\DosiTracker\dosi-tracker.exe" --quit`,
  /** Silent uninstall that keeps the sign-in, preferences and unsent time. */
  silentUninstallKeepData: String.raw`"%LOCALAPPDATA%\Programs\DosiTracker\uninstall.exe" /S /KEEPDATA`,
  verify: `Get-FileHash .\\${RELEASE_ASSET_FILES.windowsInstaller} -Algorithm SHA256`,
} as const;

export const MACOS_COMMANDS = {
  /** Set the server for every user of a Mac (or push it as an MDM profile). */
  setServer: (server: string | null) => `defaults write com.dosibridge.tracker DosiApiBaseURL "${serverOr(server)}"`,
  resetScreenRecording: "tccutil reset ScreenCapture com.dosibridge.tracker",
  verify: `shasum -a 256 ${RELEASE_ASSET_FILES.macosZip}`,
  /** Build the app from a checkout (Xcode 15+). */
  build: ["cd clients/macos", "bash scripts/build-app.sh", `cp -R ".build/dist/Dosi Tracker.app" /Applications/`],
} as const;

export const LINUX_COMMANDS = {
  installDeb: `sudo apt install ./${RELEASE_ASSET_FILES.linuxDeb}`,
  installRpm: `sudo dnf install ./${RELEASE_ASSET_FILES.linuxRpm}`,
  installTarball: (server: string | null) => [
    `tar -xzf ${RELEASE_ASSET_FILES.linuxTarball}`,
    "cd dosi-tracker-*/",
    `./install.sh --server ${serverOr(server)}`,
  ],
  verify: `sha256sum -c --ignore-missing ${RELEASE_ASSET_FILES.checksums}`,
  /** Start the agent now and at every login (installers never do this for you). */
  enable: "dosi-tracker service enable --now",
  login: (server: string | null) => `dosi-tracker login --server ${serverOr(server)}`,
  consent: "dosi-tracker consent grant --keyboard --mouse --active-window --running-programs --screenshots",
  consentTimeOnly: "dosi-tracker consent grant --time-only",
  projectUse: `dosi-tracker project use "<project>"`,
  status: "dosi-tracker status",
  pause: "dosi-tracker pause --for 30m",
  resume: "dosi-tracker resume",
  stop: "dosi-tracker stop",
  sync: "dosi-tracker sync",
  doctor: "dosi-tracker doctor",
  gui: "dosi-tracker-gui",
  revoke: "dosi-tracker consent revoke",
  logout: "dosi-tracker logout",
  uninstallTarball: "dosi-tracker uninstall",
  uninstallDeb: "sudo apt remove dosi-tracker",
  uninstallRpm: "sudo dnf remove dosi-tracker",
} as const;
