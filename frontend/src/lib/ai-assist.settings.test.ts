// The owner's AI settings rules (AI assist spec §5.3, §8, §18.6): which
// changes need the current disclosure acknowledged first, what the PUT body
// carries, the one-line state shown in Settings, the usage meters, the
// feature names of the privacy note, and the demo's settings and usage.
import { describe, expect, it } from "vitest";
import {
  AI_FEATURE_LABELS,
  aiAcknowledgementReasons,
  aiConsentCurrent,
  aiEffectivelyOn,
  aiSettingsFlags,
  aiSettingsUpdate,
  aiStatusLine,
  allowedAiFeatures,
  type AiSettingsFlags,
} from "./ai-assist";
import { AI_FEATURE_IDS, type AiStatusDto, type AiStatusReason } from "./ai-contract";
import { DEMO_DISCLOSURE_VERSION, DEMO_PROVIDER, demoAiStatus } from "./ai-demo-status";
import { demoAiSettings, demoAiUsage } from "./ai-demo-settings";
import { aiMeter, formatRate, formatUsd } from "./ai-format";

const OFF: AiSettingsFlags = { isEnabled: false, allowManagers: true, allowMemberSelfService: true, allowWindowTitles: false, allowNotes: false };
const ON: AiSettingsFlags = { ...OFF, isEnabled: true };

function status(patch: Partial<AiStatusDto> = {}): AiStatusDto {
  return { ...demoAiStatus("owner"), providerDisplayName: "Example AI", ...patch };
}

describe("aiAcknowledgementReasons", () => {
  it("asks for the disclosure when AI assist is turned on", () => {
    expect(aiAcknowledgementReasons(OFF, ON, false)).toEqual(["enable"]);
    // A first enable with no earlier acknowledgement is an enable, not a re-consent.
    expect(aiAcknowledgementReasons(OFF, ON, true)).toEqual(["enable"]);
  });

  it("asks again when saving while on with an outdated acknowledgement (re-consent)", () => {
    expect(aiAcknowledgementReasons(ON, ON, false)).toEqual(["reconsent"]);
    expect(aiAcknowledgementReasons(ON, { ...ON, allowManagers: false }, false)).toEqual(["reconsent"]);
  });

  it("asks when window titles or task notes are turned on, whether or not AI assist is on", () => {
    expect(aiAcknowledgementReasons(ON, { ...ON, allowWindowTitles: true }, true)).toEqual(["window-titles"]);
    expect(aiAcknowledgementReasons(ON, { ...ON, allowNotes: true }, true)).toEqual(["notes"]);
    expect(aiAcknowledgementReasons(OFF, { ...OFF, allowWindowTitles: true, allowNotes: true }, true)).toEqual(["window-titles", "notes"]);
    expect(aiAcknowledgementReasons(OFF, { ...ON, allowNotes: true }, false)).toEqual(["enable", "notes"]);
  });

  it("saves everything else as it is: turning off, narrowing, toggling who may use it", () => {
    expect(aiAcknowledgementReasons(ON, OFF, true)).toEqual([]);
    expect(aiAcknowledgementReasons(ON, OFF, false)).toEqual([]);
    expect(aiAcknowledgementReasons(ON, { ...ON, allowManagers: false, allowMemberSelfService: false }, true)).toEqual([]);
    const withTitles = { ...ON, allowWindowTitles: true, allowNotes: true };
    expect(aiAcknowledgementReasons(withTitles, { ...withTitles, allowWindowTitles: false, allowNotes: false }, true)).toEqual([]);
    expect(aiAcknowledgementReasons(withTitles, withTitles, true)).toEqual([]);
  });
});

describe("settings records", () => {
  const settings = demoAiSettings();

  it("reads consent as current only when the acknowledged version is the server's", () => {
    expect(aiConsentCurrent(settings)).toBe(true);
    expect(aiConsentCurrent({ ...settings, consentVersion: "0.9" })).toBe(false);
    expect(aiConsentCurrent({ ...settings, consentVersion: null })).toBe(false);
  });

  it("counts the workspace as on only when switched on with a current acknowledgement", () => {
    expect(aiEffectivelyOn(settings)).toBe(true);
    expect(aiEffectivelyOn({ ...settings, isEnabled: false })).toBe(false);
    expect(aiEffectivelyOn({ ...settings, consentVersion: "0.9" })).toBe(false);
  });

  it("builds the PUT body from the switches and the acknowledged version", () => {
    expect(aiSettingsUpdate(ON, "1.2")).toEqual({ ...ON, acknowledgedDisclosureVersion: "1.2" });
    expect(aiSettingsUpdate({ ...OFF, extra: true } as AiSettingsFlags, null)).toEqual({ ...OFF, acknowledgedDisclosureVersion: null });
  });

  it("takes only the switches from a saved record", () => {
    expect(aiSettingsFlags(settings)).toEqual({ isEnabled: true, allowManagers: true, allowMemberSelfService: true, allowWindowTitles: false, allowNotes: false });
  });
});

describe("aiStatusLine", () => {
  it("says a demo's output is sample text", () => {
    expect(aiStatusLine(status(), { demo: true })).toMatchObject({ label: "Sample", tone: "info" });
  });

  it("says the server has no AI when there is no status, it isn't configured, or the platform switched it off", () => {
    for (const s of [null, undefined, status({ available: false, reason: "not-configured" }), status({ available: false, reason: "platform-disabled" })]) {
      const line = aiStatusLine(s);
      expect(line.label).toBe("Unavailable");
      expect(line.text).toContain("isn't available on this server");
    }
  });

  it.each<[AiStatusReason, string, string]>([
    ["subscription-inactive", "Paused", "subscription"],
    ["workspace-disabled", "Off", "Nothing is sent"],
    ["reconsent-required", "Paused", "disclosure changed"],
    ["quota-exhausted", "On", "allowance is used"],
  ])("reads %s as %s", (reason, label, words) => {
    const line = aiStatusLine(status({ available: reason === "quota-exhausted", reason }));
    expect(line.label).toBe(label);
    expect(line.text).toContain(words);
  });

  it("names the processor while on, and stays quiet about one it wasn't told", () => {
    expect(aiStatusLine(status()).text).toContain("with Example AI");
    expect(aiStatusLine(status({ providerDisplayName: null })).text).not.toContain(" with ");
    expect(aiStatusLine(status({ reason: "quota-exhausted", providerDisplayName: null })).text).toBe("AI assist is on. This month's AI allowance is used.");
  });

  it("reads a member's status (no failing check, workspace off) as off", () => {
    expect(aiStatusLine(status({ reason: null, workspaceEnabled: false })).label).toBe("Off");
    // A reason the web doesn't know yet falls through to the workspace's own switch.
    expect(aiStatusLine(status({ reason: "role-not-allowed" })).label).toBe("On");
  });

  it("never names a credential (RULE 8)", () => {
    const lines = [null, ...(["not-configured", "subscription-inactive", "workspace-disabled", "reconsent-required", "quota-exhausted"] as const).map((r) => status({ reason: r }))];
    for (const s of lines) expect(aiStatusLine(s).text).not.toMatch(/secret|password|api[_-]?key/i);
  });
});

describe("allowedAiFeatures", () => {
  it("names the features the server allows, in its order", () => {
    expect(allowedAiFeatures(demoAiStatus("owner"))).toEqual(AI_FEATURE_IDS.map((id) => AI_FEATURE_LABELS[id]));
    expect(allowedAiFeatures(demoAiStatus("worker"))).not.toContain("Report explanations");
    expect(allowedAiFeatures(demoAiStatus("client"))).toEqual([]);
    expect(allowedAiFeatures(null)).toEqual([]);
  });

  it("drops a feature id the web doesn't know", () => {
    const s = status({ features: [{ featureId: "future-thing", allowed: true, reason: null, scopes: [] }] });
    expect(allowedAiFeatures(s)).toEqual([]);
  });
});

describe("meters and figures", () => {
  it("measures use against a limit, capped at 100%", () => {
    expect(aiMeter(42, 2000)).toEqual({ percent: 2, text: "42 of 2,000" });
    expect(aiMeter(2500, 2000)).toEqual({ percent: 100, text: "2,500 of 2,000" });
  });

  it("says when there is no limit, and never shows a negative or unreadable count", () => {
    expect(aiMeter(12, null)).toEqual({ percent: null, text: "12 (no monthly limit)" });
    expect(aiMeter(12, 0)).toEqual({ percent: null, text: "12 (no monthly limit)" });
    expect(aiMeter(Number.NaN, 10)).toEqual({ percent: 0, text: "0 of 10" });
    expect(aiMeter(-3, undefined).text).toBe("0 (no monthly limit)");
  });

  it("formats money and rates, with a dash for what the server couldn't say", () => {
    expect(formatUsd(12.4)).toBe("$12.40");
    expect(formatUsd(1234.5)).toBe("$1,234.50");
    expect(formatUsd(null)).toBe("—");
    expect(formatUsd(undefined)).toBe("—");
    expect(formatRate(0.125)).toBe("13%");
    expect(formatRate(0.004)).toBe("0.4%");
    expect(formatRate(0)).toBe("0%");
    expect(formatRate(0.02)).toBe("2%");
    expect(formatRate(0.0999)).toBe("10%");
    expect(formatRate(-1)).toBe("0%");
    expect(formatRate(Number.NaN)).toBe("—");
    expect(formatRate(null)).toBe("—");
  });
});

describe("demo settings and usage", () => {
  it("starts on with the code defaults and its own disclosure", () => {
    const s = demoAiSettings();
    expect(aiSettingsFlags(s)).toEqual({ isEnabled: true, allowManagers: true, allowMemberSelfService: true, allowWindowTitles: false, allowNotes: false });
    expect(s.consentVersion).toBe(DEMO_DISCLOSURE_VERSION);
    expect(s.disclosure.providerDisplayName).toBe(DEMO_PROVIDER);
    expect(s.disclosure.neverSent).toContain("Screenshots and webcam images");
    expect(s.disclosure.whoCanUse.length).toBeGreaterThan(0);
    expect(demoAiStatus("owner").providerDisplayName).toBe(DEMO_PROVIDER);
  });

  it("gives the owner workspace totals with cost, and a member their own count", () => {
    const owner = demoAiUsage("owner");
    expect(owner.requestsUsed).toBe(demoAiStatus("owner").quota?.requestsUsed);
    expect(owner.estimatedCostUsd).toBe(0);
    expect(owner.byFeature?.length).toBeGreaterThan(0);
    expect(Date.parse(owner.resetsAt)).toBeGreaterThan(Date.parse(owner.monthStart));

    const manager = demoAiUsage("admin");
    expect(manager.estimatedCostUsd).toBeNull();
    expect(manager.byFeature).toBeNull();

    const member = demoAiUsage("worker");
    expect(member.requestsLimit).toBeNull();
    expect(member.tokensLimit).toBeNull();
    expect(member.requestsUsed).toBe(3);
  });
});
