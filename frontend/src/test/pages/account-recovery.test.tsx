// FE-09 / E2E-04: account recovery and email confirmation, live. The pages
// talk to ABP's anonymous account API without a token or a workspace, never
// say whether an address has an account, and handle dead links honestly.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render as rtlRender, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import ForgotPage from "@/app/forgot-password/page";
import ResetPage from "@/app/reset-password/page";
import ConfirmEmailPage from "@/app/confirm-email/page";
import { __setPathname } from "@/test/next-navigation-stub";
import { ThemeProvider } from "@/components/theme-provider";
import { resetPrototypeState } from "@/test/harness";

/** The pages stand alone (no session needed); only the theme switch needs its provider. */
const render = (ui: React.ReactElement) => rtlRender(<ThemeProvider>{ui}</ThemeProvider>);

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const USER = "0b8a6f7e-1c2d-4e5f-8a9b-0c1d2e3f4a5b";

function reply(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
const calls = () => fetchMock.mock.calls.map(([input, init]) => ({ url: String(input).replace(API, ""), init }));
const bodyOf = (n: number) => JSON.parse(String(calls()[n].init?.body));

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("/forgot-password", () => {
  it("asks the server anonymously and answers the same for any address", async () => {
    const user = userEvent.setup();
    __setPathname("/forgot-password?email=ana%40acme.example&next=%2Fteam");
    fetchMock.mockResolvedValue(reply(undefined, 204));
    render(<ForgotPage />);

    expect(screen.getByLabelText("Email")).toHaveValue("ana@acme.example");
    await user.click(screen.getByRole("button", { name: "Email me a link" }));

    expect(await screen.findByRole("status")).toHaveTextContent("If ana@acme.example belongs to an account here");
    const [call] = calls();
    expect(call.url).toBe("/api/account/send-password-reset-code");
    expect(call.init?.method).toBe("POST");
    expect(call.init?.credentials).toBe("omit");
    expect((call.init?.headers as Record<string, string>).Authorization).toBeUndefined();
    expect((call.init?.headers as Record<string, string>)["X-Workspace-Id"]).toBeUndefined();
    expect(bodyOf(0)).toEqual({ email: "ana@acme.example", appName: "Dosi-Tracker web", returnUrl: "/team" });
  });

  it("shows the very same answer when the server refuses the address (no enumeration)", async () => {
    const user = userEvent.setup();
    fetchMock.mockResolvedValue(reply({ error: { code: "Volo.Abp.Identity:UserNotFound", message: "No such user" } }, 400));
    render(<ForgotPage />);
    await user.type(screen.getByLabelText("Email"), "nobody@nowhere.example");
    await user.click(screen.getByRole("button", { name: "Email me a link" }));
    const status = await screen.findByRole("status");
    expect(status).toHaveTextContent("If nobody@nowhere.example belongs to an account here");
    expect(status).not.toHaveTextContent(/no such user/i);
  });

  it("catches a typo before sending, and says when it couldn't send at all", async () => {
    const user = userEvent.setup();
    render(<ForgotPage />);
    await user.type(screen.getByLabelText("Email"), "ana@");
    await user.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Enter the email address you sign in with.");
    expect(fetchMock).not.toHaveBeenCalled();

    await user.clear(screen.getByLabelText("Email"));
    await user.type(screen.getByLabelText("Email"), "ana@acme.example");
    fetchMock.mockResolvedValueOnce(reply({ error: { message: "slow down" } }, 429));
    await user.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/too many requests/i);

    fetchMock.mockRejectedValueOnce(new TypeError("offline"));
    await user.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/can't reach the server/i);

    fetchMock.mockResolvedValueOnce(reply({ error: { message: "smtp down" } }, 500));
    await user.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/couldn't send the email right now/i);
  });

  it("a11y: forgot form has no violations", async () => {
    const { container } = render(<ForgotPage />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("/reset-password", () => {
  const link = `/reset-password?userId=${USER}&resetToken=tok%2B1&returnUrl=%2Fworkspaces`;

  it("checks the link, sets the new value, ends any session here and continues to sign in", async () => {
    const user = userEvent.setup();
    localStorage.setItem("dosi-token", "someone-elses-session");
    __setPathname(link);
    fetchMock.mockImplementation(async (input) => {
      const url = String(input).replace(API, "");
      if (url === "/api/account/verify-password-reset-token") return reply(true);
      if (url === "/api/account/reset-password") return reply(undefined, 204);
      throw new Error(`unexpected ${url}`);
    });
    render(<ResetPage />);

    const field = await screen.findByLabelText("New password");
    expect(bodyOf(0)).toEqual({ userId: USER, resetToken: "tok+1" });
    await user.type(field, "abc");
    await user.type(screen.getByLabelText("Confirm new password"), "abc");
    await user.click(screen.getByRole("button", { name: "Save and continue" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Use at least 6 characters.");
    expect(fetchMock).toHaveBeenCalledTimes(1);

    await user.clear(field);
    await user.type(field, "brand-new-1");
    await user.clear(screen.getByLabelText("Confirm new password"));
    await user.type(screen.getByLabelText("Confirm new password"), "brand-new-1");
    await user.click(screen.getByRole("button", { name: "Save and continue" }));

    expect(await screen.findByRole("link", { name: "Continue to sign in" })).toHaveAttribute("href", "/login?next=%2Fworkspaces");
    expect(bodyOf(1)).toEqual({ userId: USER, resetToken: "tok+1", password: "brand-new-1" });
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("says an expired link is dead before anything is typed, and offers a new one", async () => {
    __setPathname(link);
    fetchMock.mockResolvedValue(reply(false));
    render(<ResetPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/expired or was already used/i);
    expect(screen.getByRole("link", { name: "Ask for a new link" })).toHaveAttribute("href", "/forgot-password");
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
  });

  it("refuses an incomplete link without calling the server", () => {
    __setPathname("/reset-password?userId=nope");
    render(<ResetPage />);
    expect(screen.getByRole("alert")).toHaveTextContent(/incomplete/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("shows the server's policy words, or a dead link, when the reset is refused", async () => {
    const user = userEvent.setup();
    __setPathname(link);
    let resetReply = reply({ error: { message: "Passwords must have at least one digit.", validationErrors: [] } }, 400);
    fetchMock.mockImplementation(async (input) => {
      const url = String(input).replace(API, "");
      if (url === "/api/account/verify-password-reset-token") throw new TypeError("offline"); // can't check: let the reset decide
      return resetReply;
    });
    render(<ResetPage />);
    await user.type(await screen.findByLabelText("New password"), "abcdefgh");
    await user.type(screen.getByLabelText("Confirm new password"), "abcdefgh");
    await user.click(screen.getByRole("button", { name: "Save and continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Passwords must have at least one digit.");

    resetReply = reply({ error: { code: "Volo.Abp.Identity:InvalidToken", message: "Invalid token." } }, 403);
    await user.click(screen.getByRole("button", { name: "Save and continue" }));
    expect(await screen.findByRole("link", { name: "Ask for a new link" })).toBeInTheDocument();
  });

  it("a11y: reset form has no violations", async () => {
    __setPathname(link);
    fetchMock.mockResolvedValue(reply(true));
    const { container } = render(<ResetPage />);
    await screen.findByLabelText("New password");
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("/confirm-email", () => {
  it("confirms once on open and points a signed-in person at their invitations", async () => {
    localStorage.setItem("dosi-token", "jwt");
    __setPathname(`/confirm-email?userId=${USER}&token=abc`);
    fetchMock.mockResolvedValue(reply(undefined, 204));
    render(<ConfirmEmailPage />);
    expect(await screen.findByText(/your email address is confirmed/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "See your workspaces" })).toHaveAttribute("href", "/workspaces");
    expect(calls()).toHaveLength(1);
    expect(calls()[0].url).toBe("/api/app/my-account/confirm-email");
    expect(bodyOf(0)).toEqual({ userId: USER, token: "abc" });
  });

  it("explains a dead link and sends a signed-out visitor to sign in", async () => {
    __setPathname(`/confirm-email?userId=${USER}&token=old`);
    fetchMock.mockResolvedValue(reply({ error: { code: "Tracker:EmailConfirmationInvalid", message: "expired" } }, 400));
    render(<ConfirmEmailPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/expired or was already used/i);
    await waitFor(() => expect(screen.getByRole("link", { name: "Go to sign in" })).toHaveAttribute("href", "/login"));
  });

  it("refuses an incomplete link without calling the server", () => {
    __setPathname("/confirm-email");
    render(<ConfirmEmailPage />);
    expect(screen.getByRole("alert")).toHaveTextContent(/incomplete/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
