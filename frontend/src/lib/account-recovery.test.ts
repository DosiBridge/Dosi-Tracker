import { describe, expect, it } from "vitest";
import {
  MAX_NEW_CREDENTIAL_LENGTH,
  MIN_NEW_CREDENTIAL_LENGTH,
  NEW_CREDENTIAL_PROBLEM_TEXT,
  looksLikeEmail,
  newCredentialProblem,
  parseConfirmationLink,
  parseRecoveryLink,
  recoveryFailure,
  recoveryRequestedMessage,
} from "./account-recovery";

const USER = "0b8a6f7e-1c2d-4e5f-8a9b-0c1d2e3f4a5b";

describe("the reset request answer (never says whether an account exists)", () => {
  it("is one message for every address, naming what was typed", () => {
    const a = recoveryRequestedMessage("ana@acme.example");
    const b = recoveryRequestedMessage("nobody@nowhere.example");
    expect(a).toContain("If ana@acme.example belongs to an account here");
    expect(a.replace("ana@acme.example", "X")).toBe(b.replace("nobody@nowhere.example", "X"));
    expect(recoveryRequestedMessage("  ")).toContain("If that address belongs");
  });

  it("checks an address only for typos", () => {
    expect(looksLikeEmail("ana@acme.example")).toBe(true);
    expect(looksLikeEmail("  ana@acme.example ")).toBe(true);
    for (const bad of ["", "ana", "ana@", "ana@acme", "a na@acme.example", "@acme.example"]) expect(looksLikeEmail(bad)).toBe(false);
  });
});

describe("parseRecoveryLink — the emailed /reset-password link", () => {
  it("reads the account, the one-time token and a same-site return path", () => {
    expect(parseRecoveryLink(`?userId=${USER}&resetToken=CfDJ8%2Babc&returnUrl=%2Fteam`)).toEqual({
      userId: USER,
      resetToken: "CfDJ8+abc",
      returnTo: "/team",
    });
    expect(parseRecoveryLink(new URLSearchParams({ userId: USER, resetToken: "t" }))).toEqual({ userId: USER, resetToken: "t", returnTo: null });
  });

  it("never follows a return URL off this site", () => {
    for (const evil of ["https://evil.example", "//evil.example", "/\\evil.example"]) {
      expect(parseRecoveryLink(new URLSearchParams({ userId: USER, resetToken: "t", returnUrl: evil }))?.returnTo).toBeNull();
    }
  });

  it("refuses an incomplete or malformed link", () => {
    expect(parseRecoveryLink("")).toBeNull();
    expect(parseRecoveryLink(`?userId=${USER}`)).toBeNull();
    expect(parseRecoveryLink(`?userId=${USER}&resetToken=%20`)).toBeNull();
    expect(parseRecoveryLink("?userId=not-a-guid&resetToken=t")).toBeNull();
    expect(parseRecoveryLink(new URLSearchParams({ userId: USER, resetToken: "x".repeat(2049) }))).toBeNull();
  });
});

describe("parseConfirmationLink — the emailed /confirm-email link", () => {
  it("reads the account and token, and refuses an incomplete link", () => {
    expect(parseConfirmationLink(`?userId=${USER}&token=abc`)).toEqual({ userId: USER, token: "abc" });
    expect(parseConfirmationLink(`?userId=${USER}`)).toBeNull();
    expect(parseConfirmationLink("?userId=1&token=abc")).toBeNull();
    expect(parseConfirmationLink(new URLSearchParams({ userId: USER, token: "y".repeat(2049) }))).toBeNull();
  });
});

describe("newCredentialProblem — checked before anything is sent", () => {
  const ok = "a".repeat(MIN_NEW_CREDENTIAL_LENGTH);
  it.each([
    ["", "", "missing"],
    [ok, "", "missing"],
    ["abc", "abc", "short"],
    ["a".repeat(MAX_NEW_CREDENTIAL_LENGTH + 1), "a".repeat(MAX_NEW_CREDENTIAL_LENGTH + 1), "long"],
    [ok, `${ok}b`, "mismatch"],
  ] as const)("%j / %j → %s", (value, confirmation, problem) => {
    expect(newCredentialProblem(value, confirmation)).toBe(problem);
    expect(NEW_CREDENTIAL_PROBLEM_TEXT[problem]).toBeTruthy();
  });

  it("lets a long-enough matching pair go", () => {
    expect(newCredentialProblem(ok, ok)).toBeNull();
  });
});

describe("recoveryFailure — why a reset or a confirmation was refused", () => {
  it("treats a wrong or expired token as a dead link", () => {
    expect(recoveryFailure({ code: "Volo.Abp.Identity:InvalidToken" }, "x").linkDead).toBe(true);
    expect(recoveryFailure({ code: "Tracker:EmailConfirmationInvalid" }, "x").linkDead).toBe(true);
    expect(recoveryFailure({ serverMessage: "Invalid token." }, "x")).toEqual({
      linkDead: true,
      message: "This link has expired or was already used. Ask for a new one.",
    });
  });

  it("shows the policy's own words for a weak value, and keeps the link", () => {
    expect(
      recoveryFailure({ status: 400, validationErrors: [{ message: "Passwords must have at least one digit." }, { message: null }] }, "x"),
    ).toEqual({ linkDead: false, message: "Passwords must have at least one digit." });
    expect(recoveryFailure({ status: 400, serverMessage: "Too simple." }, "x")).toEqual({ linkDead: false, message: "Too simple." });
  });

  it("says when to wait, and falls back otherwise", () => {
    expect(recoveryFailure({ status: 429 }, "x").message).toMatch(/wait a minute/i);
    expect(recoveryFailure(new Error("boom"), "Fallback.")).toEqual({ linkDead: false, message: "Fallback." });
    expect(recoveryFailure(null, "Fallback.").message).toBe("Fallback.");
  });
});
