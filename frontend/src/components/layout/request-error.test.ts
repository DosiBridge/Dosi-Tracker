import { describe, expect, it } from "vitest";
import { ApiError, ApiUnreachableError } from "@/hooks/useApi";
import { FORBIDDEN_TEXT, UNREACHABLE_TEXT, requestErrorCode, requestErrorStatus, requestErrorText } from "./request-error";

const apiError = (status: number, info: Partial<{ code: string; message: string; validationErrors: { message: string; members: string[] }[] }> = {}) =>
  new ApiError(`API PUT Error: HTTP ${status}`, status, {
    code: info.code ?? null,
    message: info.message ?? null,
    details: null,
    validationErrors: info.validationErrors ?? [],
  });

describe("requestErrorText", () => {
  it("lists the server's validation messages first", () => {
    const err = apiError(400, {
      message: "Your request is not valid!",
      validationErrors: [
        { message: "The Email field is not a valid e-mail address.", members: ["email"] },
        { message: "The Name field is required.", members: ["name"] },
      ],
    });
    expect(requestErrorText(err, "x")).toBe("The Email field is not a valid e-mail address. The Name field is required.");
  });

  it("uses the server's own message when there are no validation errors", () => {
    expect(requestErrorText(apiError(409, { message: "Email 'a@b.c' is already taken." }), "x")).toBe("Email 'a@b.c' is already taken.");
  });

  it("explains a bare 403 and gives other bare failures the fallback with their status — never 'API PUT Error: Forbidden'", () => {
    expect(requestErrorText(apiError(403), "Couldn't save.")).toBe(FORBIDDEN_TEXT);
    expect(requestErrorText(apiError(500), "Couldn't save.")).toBe("Couldn't save. (HTTP 500)");
  });

  it("says the server couldn't be reached when the request never got an answer", () => {
    expect(requestErrorText(new TypeError("Failed to fetch"), "x")).toBe(UNREACHABLE_TEXT);
    expect(requestErrorText(new ApiUnreachableError(), "x")).toBe(UNREACHABLE_TEXT);
  });

  it("falls back for anything else", () => {
    expect(requestErrorText(new Error("boom"), "Couldn't save.")).toBe("Couldn't save.");
    expect(requestErrorText("weird", "Couldn't save.")).toBe("Couldn't save.");
  });
});

describe("requestErrorCode / requestErrorStatus", () => {
  it("read an ApiError's business code and status, null otherwise", () => {
    const err = apiError(402, { code: "Tracker:SubscriptionInactive" });
    expect(requestErrorCode(err)).toBe("Tracker:SubscriptionInactive");
    expect(requestErrorStatus(err)).toBe(402);
    expect(requestErrorCode(new Error("x"))).toBeNull();
    expect(requestErrorStatus(new TypeError("Failed to fetch"))).toBeNull();
  });
});
