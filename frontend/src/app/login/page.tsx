"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Radar, Eye, EyeOff, Loader2, ArrowLeft, Building2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { LoginHero } from "@/components/motion/login-hero";
import { loginApi, registerWorkspaceApi } from "@/hooks/useApi";
import type { PlanId } from "@/lib/saas-data";

// Backend plan names (seeded by PlanDataSeedContributor).
const PLAN_NAME: Record<PlanId, string> = {
  free: "Free",
  starter: "Starter",
  business: "Business",
  enterprise: "Business",
};

export default function LoginPage() {
  const router = useRouter();
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [show, setShow] = useState(false);
  const [loading, setLoading] = useState<string | null>(null);
  const [wsName, setWsName] = useState("");
  const [wsPlan, setWsPlan] = useState<PlanId>("free");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading("signin");
    setErrorMsg(null);
    try {
      const tenant = workspace.trim();
      // Workspace (tenant) name scopes the login; leave empty for the host account.
      await loginApi(email, password, tenant || undefined);
      // A host sign-in (no tenant) lands on the platform console; tenant users on the dashboard.
      const destination = tenant ? "/dashboard" : "/host";
      setTimeout(() => router.push(destination), 500);
    } catch {
      setErrorMsg("Invalid username, password or workspace");
      setLoading(null);
    }
  }

  async function onCreateWorkspace(e: React.FormEvent) {
    e.preventDefault();
    if (!wsName.trim() || !password) return;
    setLoading("__signup__");
    setErrorMsg(null);
    try {
      const result = await registerWorkspaceApi(wsName.trim(), email, password, PLAN_NAME[wsPlan]);
      // Log straight into the freshly created tenant as its admin.
      await loginApi(email, password, result.name);
      setTimeout(() => router.push("/dashboard"), 600);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Registration failed");
      setLoading(null);
    }
  }

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

          {mode === "signup" ? (
            <>
              <button
                onClick={() => setMode("signin")}
                className="mb-4 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
              >
                <ArrowLeft className="h-4 w-4" /> Back to sign in
              </button>
              <h2 className="font-display text-2xl font-bold tracking-tight">Create your workspace</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Spin up a new, fully isolated tenant. You&apos;ll be the owner.
              </p>

              <form onSubmit={onCreateWorkspace} className="mt-8 space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="signup-email" className="text-sm font-medium">Email</label>
                  <Input id="signup-email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" required autoFocus />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-password" className="text-sm font-medium">Password</label>
                  <Input id="signup-password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={6} />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="signup-workspace-name" className="text-sm font-medium">Workspace name</label>
                  <Input id="signup-workspace-name" value={wsName} onChange={(e) => setWsName(e.target.value)} placeholder="Acme Corp" required />
                  {wsName && (
                    <p className="text-xs text-muted-foreground">
                      {wsName.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "workspace"}.dositracker.app
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
                {errorMsg && <p className="text-sm text-red-500 font-medium">{errorMsg}</p>}
                <Button type="submit" size="lg" className="w-full" disabled={!!loading || !wsName.trim() || !email || !password}>
                  {loading === "__signup__" && <Loader2 className="h-4 w-4 animate-spin" />}
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
                    id="login-workspace"
                    value={workspace}
                    onChange={(e) => setWorkspace(e.target.value)}
                    placeholder="acme (leave empty for host sign-in)"
                  />
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="login-email" className="text-sm font-medium">Email</label>
                  <Input id="login-email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
                </div>
                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <label htmlFor="login-password" className="text-sm font-medium">Password</label>
                    <button type="button" className="text-xs text-primary-strong hover:underline">
                      Forgot password?
                    </button>
                  </div>
                  <div className="relative">
                    <Input
                      id="login-password"
                      type={show ? "text" : "password"}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      required
                      className="pr-10"
                    />
                    <button
                      type="button"
                      onClick={() => setShow((s) => !s)}
                      aria-label={show ? "Hide password" : "Show password"}
                      className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    >
                      {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>

                {errorMsg && <p className="text-sm text-red-500 font-medium">{errorMsg}</p>}
                <Button type="submit" size="lg" className="w-full" disabled={!!loading}>
                  {loading && <Loader2 className="h-4 w-4 animate-spin" />}
                  {loading ? "Signing in…" : "Sign in"}
                </Button>
              </form>

              <div className="mt-8">
                <div className="mt-6 rounded-xl border border-dashed border-border p-3 text-center">
                  <p className="text-sm text-muted-foreground">New to Dosi-Tracker?</p>
                  <button
                    onClick={() => setMode("signup")}
                    className="mt-1 text-sm font-semibold text-primary-strong hover:underline"
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
