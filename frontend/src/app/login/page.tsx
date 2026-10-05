"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useRouter } from "next/navigation";
import { Radar, Eye, EyeOff, Loader2, ArrowLeft, Building2, CheckCircle2, KeyRound, ShieldAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { LoginHero } from "@/components/motion/login-hero";
import { loginApi, registerWorkspaceApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { landingFor } from "@/lib/roles";
import { SUPPORT_EMAIL } from "@/lib/brand";
import { isStorageAvailable } from "@/lib/live-session";
import {
  WEB_SIGN_IN_CLIENT,
  changePasswordAtSignIn,
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
  tenant: string | undefined;
}

export default function LoginPage() {
  const router = useRouter();
  const { refreshSession } = useSession();
  const [mode, setModeState] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [show, setShow] = useState(false);
  const [loading, setLoading] = useState<string | null>(null);
  const [wsName, setWsName] = useState("");
  const [wsPlan, setWsPlan] = useState<PlanId>("free");
  const [failure, setFailure] = useState<SignInFailure | null>(null);
  /** The workspace name the failure refers to (as typed when it happened). */
  const [failedWorkspace, setFailedWorkspace] = useState("");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [infoMsg, setInfoMsg] = useState<string | null>(null);
  /** Set after a successful signup: the exact workspace name to sign in with. */
  const [createdWorkspace, setCreatedWorkspace] = useState<string | null>(null);
  /** A rate limit: no sign-in attempts before this instant (epoch ms). */
  const [retryAt, setRetryAt] = useState<number | null>(null);
  const [now, setNow] = useState<number | null>(null);
  const [pendingChange, setPendingChange] = useState<PendingChange | null>(null);
  const [newSecret, setNewSecret] = useState("");
  const [confirmSecret, setConfirmSecret] = useState("");
  const workspaceRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);

  // Blocked site data means no session can be kept: say so before anyone types a password.
  // (On the server, and while hydrating, assume it works — the notice only appears in a browser that refuses.)
  const storageAvailable = useSyncExternalStore(subscribeNothing, isStorageAvailable, assumeAvailable);

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

  // Switching between sign-in and signup starts clean: an error from one form
  // must not linger over the other.
  function setMode(next: "signin" | "signup") {
    setModeState(next);
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
  }

  /** Show a failure, start a rate limit's countdown, and point at the field to fix. */
  function applyFailure(f: SignInFailure, typedWorkspace: string) {
    setFailure(f);
    setFailedWorkspace(typedWorkspace);
    if (f.kind === "rate-limited") setRetryAt(Date.now() + (f.retryAfterSeconds ?? 60) * 1000);
    const field = failureField(f);
    if (field === "workspace") workspaceRef.current?.focus();
    else if (field === "credentials") passwordRef.current?.select();
  }

  function fail(err: unknown, typedWorkspace: string): SignInFailure {
    const f = classifySignInError(err);
    applyFailure(f, typedWorkspace);
    return f;
  }

  async function finishSignIn(tenant: string) {
    // Build the real session (identity, workspace, data) BEFORE navigating,
    // so the first page after sign-in already shows this account, not a
    // stale or demo one. The role decides where they land.
    const signedIn = await refreshSession();
    const destination = signedIn ? landingFor(signedIn.role) : tenant ? "/dashboard" : "/host";
    router.push(destination);
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (rateLimited) return;
    setLoading("signin");
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
    const tenant = workspace.trim();
    try {
      // Workspace (tenant) name scopes the login; the platform account signs in without one.
      await loginApi(email, password, tenant || undefined);
    } catch (err) {
      const f = fail(err, tenant);
      if (f.kind === "change-required") {
        setPendingChange({ email, current: password, tenant: tenant || undefined });
        setFailure(null);
      }
      setLoading(null);
      return;
    }
    await finishSignIn(tenant);
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
      tokenUrl: tokenEndpoint(pendingChange.tenant),
      clientId: WEB_SIGN_IN_CLIENT.clientId,
      scope: WEB_SIGN_IN_CLIENT.scope,
      username: pendingChange.email,
      currentPassword: pendingChange.current,
      newPassword: newSecret,
    });
    if (!result.ok) {
      // The server's own words for a refused new password (its rules); otherwise the usual explanation.
      if (result.message) setErrorMsg(result.message);
      else applyFailure(result.failure, pendingChange.tenant ?? "");
      setLoading(null);
      return;
    }
    try {
      await loginApi(pendingChange.email, newSecret, pendingChange.tenant);
    } catch (err) {
      fail(err, pendingChange.tenant ?? "");
      setLoading(null);
      return;
    }
    setPendingChange(null);
    setPassword("");
    await finishSignIn(pendingChange.tenant ?? "");
  }

  async function onCreateWorkspace(e: React.FormEvent) {
    e.preventDefault();
    if (!wsName.trim() || !password) return;
    setLoading("__signup__");
    setFailure(null);
    setErrorMsg(null);
    setInfoMsg(null);
    let created: string;
    try {
      const result = await registerWorkspaceApi(wsName.trim(), email, password, PLAN_NAME[wsPlan]);
      created = result.name || wsName.trim();
    } catch (err) {
      const f = classifySignInError(err);
      setErrorMsg(
        f.kind === "unreachable"
          ? signInFailureMessage(f)
          : err instanceof Error
            ? err.message
            : "We couldn't create your workspace. Please try again.",
      );
      setLoading(null);
      return;
    }
    try {
      // Log straight into the freshly created tenant as its admin.
      await loginApi(email, password, created);
      await refreshSession();
    } catch (err) {
      // The workspace exists now — don't strand it behind a "name already taken"
      // on the next attempt: switch to sign-in with everything filled in.
      setModeState("signin");
      setWorkspace(created);
      setLoading(null);
      setInfoMsg(`Your workspace “${created}” was created. Sign in to continue.`);
      fail(err, created);
      return;
    }
    // Show the exact name they will type at the next sign-in before moving on.
    setCreatedWorkspace(created);
    setLoading(null);
  }

  const errorText = errorMsg ?? (failure ? signInFailureMessage(failure, { workspace: failedWorkspace }) : null);
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

          {createdWorkspace ? (
            <div className="space-y-5">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-success/15 text-success">
                <CheckCircle2 className="h-6 w-6" />
              </div>
              <div>
                <h2 className="font-display text-2xl font-bold tracking-tight">Your workspace is ready</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  You&apos;re signed in as its owner. Next time you sign in, enter this in the Workspace field:
                </p>
              </div>
              <div className="rounded-xl border border-border bg-muted/40 p-3">
                <div className="text-xs text-muted-foreground">Workspace</div>
                <div className="mt-0.5 break-all font-mono text-base font-semibold" data-testid="created-workspace-name">
                  {createdWorkspace}
                </div>
              </div>
              <Button size="lg" className="w-full" onClick={() => router.push("/dashboard")}>
                Continue to your dashboard
              </Button>
            </div>
          ) : pendingChange ? (
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
                This account has to set a new password before it can sign in — your workspace owner gave you a one-time
                password, or yours has expired. You&apos;ll use the new one from now on, here and in the desktop app.
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
              <h2 className="font-display text-2xl font-bold tracking-tight">Create your workspace</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Spin up a new, fully isolated tenant. You&apos;ll be the owner.
              </p>

              <form onSubmit={onCreateWorkspace} className="mt-8 space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="signup-email" className="text-sm font-medium">Email</label>
                  <Input id="signup-email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" autoComplete="email" required autoFocus />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-password" className="text-sm font-medium">Password</label>
                  <Input id="signup-password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required minLength={6} />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-workspace-name" className="text-sm font-medium">Workspace name</label>
                  <Input id="signup-workspace-name" value={wsName} onChange={(e) => setWsName(e.target.value)} placeholder="Acme Corp" autoComplete="organization" required />
                  {wsName.trim() && (
                    <p className="text-xs text-muted-foreground">
                      You&apos;ll sign in with the workspace name <span className="font-medium text-foreground">{wsName.trim()}</span>.
                    </p>
                  )}
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-plan" className="text-sm font-medium">Choose a plan</label>
                  <Select id="signup-plan" value={wsPlan} onChange={(e) => setWsPlan(e.target.value as PlanId)}>
                    <option value="free">Free — $0 · up to 3 seats</option>
                    <option value="starter">Starter — $6/user/mo · 14-day trial</option>
                    <option value="business">Business — $12/user/mo · 14-day trial</option>
                  </Select>
                </div>
                {errorText && <ErrorNote id={errorId}>{errorText}</ErrorNote>}
                <Button type="submit" size="lg" className="w-full" disabled={!!loading || !storageAvailable || !wsName.trim() || !email || !password}>
                  {loading === "__signup__" && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  {loading === "__signup__" ? "Creating…" : "Create workspace"}
                </Button>
              </form>

              <div className="mt-6 flex items-start gap-2.5 rounded-xl border border-border bg-muted/40 p-3 text-xs text-muted-foreground">
                <Building2 className="mt-0.5 h-4 w-4 shrink-0" />
                Every workspace is a separate tenant with isolated members, projects, screenshots, and billing.
              </div>
            </>
          ) : (
            <>
              <h2 className="font-display text-2xl font-bold tracking-tight">Welcome back</h2>
              <p className="mt-1 text-sm text-muted-foreground">Sign in to your workspace to continue.</p>

              <form onSubmit={onSubmit} className="mt-8 space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="login-workspace" className="text-sm font-medium">Workspace</label>
                  <Input
                    ref={workspaceRef}
                    id="login-workspace"
                    value={workspace}
                    onChange={(e) => setWorkspace(e.target.value)}
                    placeholder="Your workspace name"
                    autoComplete="organization"
                    aria-invalid={failure?.kind === "unknown-workspace" ? true : undefined}
                    aria-describedby={failure?.kind === "unknown-workspace" ? `${errorId} login-workspace-hint` : "login-workspace-hint"}
                  />
                  <p id="login-workspace-hint" className="text-xs text-muted-foreground">The name your workspace was created with.</p>
                </div>
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
                    Create a workspace →
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
