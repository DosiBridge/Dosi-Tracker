"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useRouter } from "next/navigation";
import { Radar, Eye, EyeOff, Loader2, ArrowLeft, Building2, KeyRound, ShieldAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { LoginHero } from "@/components/motion/login-hero";
import { ApiError, loginApi, registerWorkspaceApi } from "@/hooks/useApi";
import { resolveLanding, safeNext } from "@/hooks/workspace-landing";
import { useSession } from "@/components/session-provider";
import { landingFor } from "@/lib/roles";
import { SUPPORT_EMAIL } from "@/lib/brand";
import { isStorageAvailable, setCurrentWorkspace } from "@/lib/live-session";
import {
  WEB_SIGN_IN_CLIENT,
  changePasswordAtSignIn,
  EMAIL_ALREADY_REGISTERED_MESSAGE,
  classifySignInError,
  failureField,
  signInFailureMessage,
  tokenEndpoint,
  type SignInFailure,
} from "@/lib/sign-in";
import type { PlanId } from "@/lib/saas-data";

// Backend plan names (seeded by PlanDataSeedContributor).
const PLAN_NAME: Record<PlanId, string> = {
  free: "Free",
  starter: "Starter",
  business: "Business",
  enterprise: "Business",
};

/** ABP Identity's default minimum; the server's own rules are reported if it asks for more. */
const MIN_NEW_SECRET_LENGTH = 6;

const deadlineAfter = (seconds: number) => Date.now() + seconds * 1000;
const subscribeNothing = () => () => {};
const assumeAvailable = () => true;

/** "0:45" */
function clock(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The account that must replace its password before it can sign in. */
interface PendingChange {
  email: string;
  current: string;
}

export default function LoginPage() {
  const router = useRouter();
  const { refreshSession } = useSession();
  const [mode, setModeState] = useState<"signin" | "signup">("signin");
  const [firstName, setFirstName] = useState("");
  const [surname, setSurname] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [loading, setLoading] = useState<string | null>(null);
  const [wsName, setWsName] = useState("");
  const [wsPlan, setWsPlan] = useState<PlanId>("free");
  const [failure, setFailure] = useState<SignInFailure | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [infoMsg, setInfoMsg] = useState<string | null>(null);
  /** Joining by invitation (`?invite=`): sign-up creates no workspace. */
  const [inviteToken, setInviteToken] = useState<string | null>(null);
  /** Where to go after signing in (`?next=`, e.g. back to an invitation). */
  const [next, setNext] = useState<string | null>(null);
  /** A rate limit: no sign-in attempts before this instant (epoch ms). */
  const [retryAt, setRetryAt] = useState<number | null>(null);
  const [now, setNow] = useState<number | null>(null);
  const [pendingChange, setPendingChange] = useState<PendingChange | null>(null);
  const [newSecret, setNewSecret] = useState("");
  const [confirmSecret, setConfirmSecret] = useState("");
  const passwordRef = useRef<HTMLInputElement>(null);

  // Blocked site data means no session can be kept: say so before anyone types a password.
  const storageAvailable = useSyncExternalStore(subscribeNothing, isStorageAvailable, assumeAvailable);

  // ?mode=signup&email=…&invite=…&next=…
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const id = window.setTimeout(() => {
      setNext(safeNext(params.get("next")));
      const invite = params.get("invite");
      if (invite) setInviteToken(invite);
      const prefill = params.get("email");
      if (prefill) setEmail(prefill);
      if (params.get("mode") === "signup") setModeState("signup");
    }, 0);
    return () => window.clearTimeout(id);
  }, []);

  // Count down a rate limit (on the button), then let the form go again.
  useEffect(() => {
    if (retryAt === null) return;
    const tick = () => {
      const t = Date.now();
      setNow(t);
      if (t >= retryAt) {
        setRetryAt(null);
        setFailure((f) => (f?.kind === "rate-limited" ? null : f));
      }
    };
    const id = window.setInterval(tick, 1000);
    const first = window.setTimeout(tick, 0);
    return () => {
      window.clearInterval(id);
      window.clearTimeout(first);
    };
  }, [retryAt]);

  const secondsLeft = retryAt !== null && now !== null ? Math.max(0, (retryAt - now) / 1000) : null;
  const rateLimited = retryAt !== null;

  function setMode(nextMode: "signin" | "signup") {
    setModeState(nextMode);
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
  }

  function applyFailure(f: SignInFailure) {
    setFailure(f);
    if (f.kind === "rate-limited") setRetryAt(deadlineAfter(f.retryAfterSeconds ?? 60));
    if (failureField(f) === "credentials") passwordRef.current?.select();
  }

  function fail(err: unknown): SignInFailure {
    const f = classifySignInError(err);
    applyFailure(f);
    return f;
  }

  /** Signed in: pick the workspace (or the console, or the chooser) and go. */
  async function finishSignIn(workspaceId?: string) {
    if (next) {
      router.push(next);
      return;
    }
    let landing;
    try {
      if (workspaceId) {
        setCurrentWorkspace(workspaceId);
        landing = "dashboard" as const;
      } else {
        landing = await resolveLanding();
      }
    } catch (err) {
      fail(err);
      setLoading(null);
      return;
    }
    if (landing === "chooser") {
      router.push("/workspaces");
      return;
    }
    // Build the real session BEFORE navigating, so the first page already shows this account.
    const signedIn = await refreshSession();
    router.push(signedIn ? landingFor(signedIn.role) : landing === "host" ? "/host" : "/dashboard");
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (rateLimited) return;
    setLoading("signin");
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
    try {
      await loginApi(email.trim(), password);
    } catch (err) {
      const f = fail(err);
      if (f.kind === "change-required") {
        setPendingChange({ email: email.trim(), current: password });
        setFailure(null);
      }
      setLoading(null);
      return;
    }
    await finishSignIn();
  }

  async function onChangeSecret(e: React.FormEvent) {
    e.preventDefault();
    if (!pendingChange || rateLimited) return;
    setFailure(null);
    setErrorMsg(null);
    if (newSecret.length < MIN_NEW_SECRET_LENGTH) {
      setErrorMsg(`Use at least ${MIN_NEW_SECRET_LENGTH} characters.`);
      return;
    }
    if (newSecret !== confirmSecret) {
      setErrorMsg("The two entries don't match.");
      return;
    }
    if (newSecret === pendingChange.current) {
      setErrorMsg("Choose something different from what you signed in with.");
      return;
    }
    setLoading("change");
    const result = await changePasswordAtSignIn({
      fetch: (input, init) => fetch(input, init),
      tokenUrl: tokenEndpoint(),
      clientId: WEB_SIGN_IN_CLIENT.clientId,
      scope: WEB_SIGN_IN_CLIENT.scope,
      username: pendingChange.email,
      currentPassword: pendingChange.current,
      newPassword: newSecret,
    });
    if (!result.ok) {
      if (result.message) setErrorMsg(result.message);
      else applyFailure(result.failure);
      setLoading(null);
      return;
    }
    try {
      await loginApi(pendingChange.email, newSecret);
    } catch (err) {
      fail(err);
      setLoading(null);
      return;
    }
    setPendingChange(null);
    setPassword("");
    await finishSignIn();
  }

  async function onSignUp(e: React.FormEvent) {
    e.preventDefault();
    if ((!inviteToken && !wsName.trim()) || !password || !email.trim()) return;
    setLoading("__signup__");
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
    let workspaceId: string | undefined;
    try {
      const result = await registerWorkspaceApi({
        firstName: firstName.trim() || undefined,
        surname: surname.trim() || undefined,
        email: email.trim(),
        password,
        ...(inviteToken ? { inviteToken } : { workspaceName: wsName.trim(), planName: PLAN_NAME[wsPlan] }),
      });
      workspaceId = result?.workspaceId || undefined;
    } catch (err) {
      if (err instanceof ApiError && err.code === "Tracker:EmailAlreadyRegistered") {
        setModeState("signin");
        setInfoMsg(EMAIL_ALREADY_REGISTERED_MESSAGE);
      } else if (err instanceof ApiError && err.code === "Tracker:DuplicateWorkspaceName") {
        setErrorMsg("A workspace with that name already exists. Choose another name.");
      } else {
        const f = classifySignInError(err);
        setErrorMsg(
          f.kind === "unreachable"
            ? signInFailureMessage(f)
            : err instanceof ApiError && err.serverMessage
              ? err.serverMessage
              : "We couldn't create your account. Please try again.",
        );
      }
      setLoading(null);
      return;
    }
    try {
      await loginApi(email.trim(), password);
    } catch (err) {
      setModeState("signin");
      setInfoMsg("Your account was created. Sign in to continue.");
      fail(err);
      setLoading(null);
      return;
    }
    await finishSignIn(workspaceId);
  }

  const errorText = errorMsg ?? (failure ? signInFailureMessage(failure) : null);
  const errorId = "login-error";
  const describedBy = errorText ? errorId : undefined;
  const credentialsInvalid = failure?.kind === "invalid";

  return (
    <div className="grid min-h-screen lg:grid-cols-2">
      <LoginHero />

      <div className="relative flex items-center justify-center overflow-y-auto p-6">
        <div className="absolute right-4 top-4 z-10 animate-fade-in">
          <ThemeToggle />
        </div>
        <div className="page-enter w-full max-w-sm py-10">
          <div className="mb-8 flex items-center gap-3 lg:hidden">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl brand-gradient text-white">
              <Radar className="h-6 w-6" />
            </div>
            <span className="font-display text-lg font-bold tracking-tight">Dosi-Tracker</span>
          </div>

          {!storageAvailable && (
            <div role="alert" className="mb-6 flex items-start gap-2.5 rounded-xl border border-warning/40 bg-warning/10 p-3 text-sm text-warning-on-tint">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
              <span>
                This browser is blocking site data for this site, so signing in can&apos;t work here. Allow cookies and site
                data for this site (or leave private browsing), then reload the page.
              </span>
            </div>
          )}

          {pendingChange ? (
            <>
              <button
                type="button"
                onClick={() => {
                  setPendingChange(null);
                  setNewSecret("");
                  setConfirmSecret("");
                  setErrorMsg(null);
                  setFailure(null);
                }}
                className="mb-4 inline-flex items-center gap-1.5 rounded text-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ArrowLeft className="h-4 w-4" aria-hidden="true" /> Back to sign in
              </button>
              <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
                <KeyRound className="h-5 w-5" aria-hidden="true" />
              </div>
              <h2 className="mt-4 font-display text-2xl font-bold tracking-tight">Choose a new password</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                This account has to set a new password before it can sign in. You&apos;ll use the new one from now on, here
                and in the desktop app.
              </p>
              <form onSubmit={onChangeSecret} className="mt-8 space-y-4" noValidate>
                <div className="space-y-1.5">
                  <label htmlFor="login-new-pw" className="text-sm font-medium">
                    New password
                  </label>
                  <Input
                    id="login-new-pw"
                    type="password"
                    value={newSecret}
                    onChange={(e) => setNewSecret(e.target.value)}
                    autoComplete="new-password"
                    minLength={MIN_NEW_SECRET_LENGTH}
                    required
                    autoFocus
                    aria-describedby={describedBy ? `${describedBy} login-new-pw-hint` : "login-new-pw-hint"}
                  />
                  <p id="login-new-pw-hint" className="text-xs text-muted-foreground">
                    At least {MIN_NEW_SECRET_LENGTH} characters.
                  </p>
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="login-confirm-pw" className="text-sm font-medium">
                    Confirm new password
                  </label>
                  <Input
                    id="login-confirm-pw"
                    type="password"
                    value={confirmSecret}
                    onChange={(e) => setConfirmSecret(e.target.value)}
                    autoComplete="new-password"
                    required
                    aria-describedby={describedBy}
                  />
                </div>
                {errorText && <ErrorNote id={errorId}>{errorText}</ErrorNote>}
                <Button type="submit" size="lg" className="w-full" disabled={!!loading || rateLimited || !newSecret || !confirmSecret}>
                  {loading === "change" && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  {loading === "change" ? "Saving…" : rateLimited && secondsLeft !== null ? `Try again in ${clock(secondsLeft)}` : "Save and sign in"}
                </Button>
              </form>
            </>
          ) : mode === "signup" ? (
            <>
              <button
                type="button"
                onClick={() => setMode("signin")}
                className="mb-4 inline-flex items-center gap-1.5 rounded text-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ArrowLeft className="h-4 w-4" aria-hidden="true" /> Back to sign in
              </button>
              <h2 className="font-display text-2xl font-bold tracking-tight">{inviteToken ? "Create your account" : "Create your workspace"}</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {inviteToken
                  ? "One account for every workspace you join."
                  : "You'll be its owner. One account works across all your workspaces."}
              </p>

              <form onSubmit={onSignUp} className="mt-8 space-y-4">
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <label htmlFor="signup-first-name" className="text-sm font-medium">First name</label>
                    <Input id="signup-first-name" value={firstName} onChange={(e) => setFirstName(e.target.value)} autoComplete="given-name" autoFocus />
                  </div>
                  <div className="space-y-1.5">
                    <label htmlFor="signup-surname" className="text-sm font-medium">Surname</label>
                    <Input id="signup-surname" value={surname} onChange={(e) => setSurname(e.target.value)} autoComplete="family-name" />
                  </div>
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-email" className="text-sm font-medium">Email</label>
                  <Input id="signup-email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" autoComplete="email" required />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-password" className="text-sm font-medium">Password</label>
                  <Input id="signup-password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required minLength={6} />
                </div>
                {!inviteToken && (
                  <>
                    <div className="space-y-1.5">
                      <label htmlFor="signup-workspace-name" className="text-sm font-medium">Workspace name</label>
                      <Input id="signup-workspace-name" value={wsName} onChange={(e) => setWsName(e.target.value)} placeholder="Acme Corp" autoComplete="organization" required />
                    </div>
                    <div className="space-y-1.5">
                      <label htmlFor="signup-plan" className="text-sm font-medium">Choose a plan</label>
                      <Select id="signup-plan" value={wsPlan} onChange={(e) => setWsPlan(e.target.value as PlanId)}>
                        <option value="free">Free — $0 · up to 3 seats</option>
                        <option value="starter">Starter — $6/user/mo · 14-day trial</option>
                        <option value="business">Business — $12/user/mo · 14-day trial</option>
                      </Select>
                    </div>
                  </>
                )}
                {errorText && <ErrorNote id={errorId}>{errorText}</ErrorNote>}
                <Button
                  type="submit"
                  size="lg"
                  className="w-full"
                  disabled={!!loading || !storageAvailable || (!inviteToken && !wsName.trim()) || !email || !password}
                >
                  {loading === "__signup__" && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  {loading === "__signup__" ? "Creating…" : inviteToken ? "Create account" : "Create workspace"}
                </Button>
              </form>

              {!inviteToken && (
                <div className="mt-6 flex items-start gap-2.5 rounded-xl border border-border bg-muted/40 p-3 text-xs text-muted-foreground">
                  <Building2 className="mt-0.5 h-4 w-4 shrink-0" />
                  Every workspace keeps its own members, projects, screenshots and billing.
                </div>
              )}
            </>
          ) : (
            <>
              <h2 className="font-display text-2xl font-bold tracking-tight">Welcome back</h2>
              <p className="mt-1 text-sm text-muted-foreground">Sign in with your email and password.</p>

              <form onSubmit={onSubmit} className="mt-8 space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="login-email" className="text-sm font-medium">Email</label>
                  <Input
                    id="login-email"
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    autoComplete="username"
                    required
                    aria-invalid={credentialsInvalid ? true : undefined}
                    aria-describedby={credentialsInvalid ? errorId : undefined}
                  />
                </div>
                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <label htmlFor="login-password" className="text-sm font-medium">Password</label>
                    <button
                      type="button"
                      onClick={() => {
                        setErrorMsg(null);
                        setFailure(null);
                        setInfoMsg(
                          `Contact your workspace owner to reset your password. Workspace owners: email ${SUPPORT_EMAIL}.`,
                        );
                      }}
                      className="rounded text-xs text-primary-strong hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      Forgot password?
                    </button>
                  </div>
                  <div className="relative">
                    <Input
                      ref={passwordRef}
                      id="login-password"
                      type={show ? "text" : "password"}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      autoComplete="current-password"
                      required
                      className="pr-10"
                      aria-invalid={credentialsInvalid ? true : undefined}
                      aria-describedby={credentialsInvalid ? errorId : undefined}
                    />
                    <button
                      type="button"
                      onClick={() => setShow((s) => !s)}
                      aria-label={show ? "Hide password" : "Show password"}
                      className="absolute right-3 top-1/2 -translate-y-1/2 rounded text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>

                {infoMsg && (
                  <p role="status" className="rounded-lg bg-muted px-3 py-2 text-sm text-muted-foreground">{infoMsg}</p>
                )}
                {errorText && <ErrorNote id={errorId}>{errorText}</ErrorNote>}
                <Button type="submit" size="lg" className="w-full" disabled={!!loading || rateLimited || !storageAvailable}>
                  {loading && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  {loading ? "Signing in…" : rateLimited && secondsLeft !== null ? `Try again in ${clock(secondsLeft)}` : "Sign in"}
                </Button>
              </form>

              <div className="mt-8">
                <div className="mt-6 rounded-xl border border-dashed border-border p-3 text-center">
                  <p className="text-sm text-muted-foreground">New to Dosi-Tracker?</p>
                  <button
                    type="button"
                    onClick={() => setMode("signup")}
                    className="mt-1 rounded text-sm font-semibold text-primary-strong hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {inviteToken ? "Create an account →" : "Create a workspace →"}
                  </button>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** A sign-in problem, announced to screen readers and readable at AA contrast on its tint. */
function ErrorNote({ id, children }: { id: string; children: React.ReactNode }) {
  return (
    <p id={id} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
      {children}
    </p>
  );
}
