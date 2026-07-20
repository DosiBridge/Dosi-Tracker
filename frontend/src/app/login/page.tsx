"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Radar, Eye, EyeOff, Loader2, ChevronRight, ArrowLeft, Building2, Server } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { LoginHero } from "@/components/motion/login-hero";
import { useSession } from "@/components/session-provider";
import { users } from "@/lib/mock-data";
import { landingFor, roleLabels, roleDescriptions } from "@/lib/roles";
import type { PlanId } from "@/lib/saas-data";
import type { Role } from "@/lib/types";

const roleTone: Record<Role, "primary" | "success" | "info" | "warning"> = {
  host: "primary",
  owner: "primary",
  admin: "warning",
  worker: "success",
  client: "info",
};

const roleOrder: Role[] = ["owner", "admin", "worker", "client"];
const demoAccounts = [...users].sort(
  (a, b) => roleOrder.indexOf(a.role) - roleOrder.indexOf(b.role)
);

export default function LoginPage() {
  const router = useRouter();
  const { setUserById, createWorkspace, loginAsHost } = useSession();
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("ayesha@dosi.dev");
  const [password, setPassword] = useState("demo1234");
  const [show, setShow] = useState(false);
  const [loading, setLoading] = useState<string | null>(null);
  const [wsName, setWsName] = useState("");
  const [wsPlan, setWsPlan] = useState<PlanId>("starter");

  function signInAs(userId: string) {
    const user = users.find((u) => u.id === userId);
    if (!user) return;
    setLoading(userId);
    setUserById(userId);
    setTimeout(() => router.push(landingFor(user.role)), 500);
  }

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const match = users.find((u) => u.email.toLowerCase() === email.trim().toLowerCase());
    signInAs((match ?? users[0]).id);
  }

  function signInAsHost() {
    setLoading("__host__");
    loginAsHost();
    setTimeout(() => router.push("/host"), 500);
  }

  function onCreateWorkspace(e: React.FormEvent) {
    e.preventDefault();
    if (!wsName.trim()) return;
    setLoading("__signup__");
    const ws = createWorkspace(wsName.trim(), wsPlan);
    setUserById(`${ws.id}-owner`);
    setTimeout(() => router.push("/dashboard"), 600);
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
                  <label className="text-sm font-medium">Workspace name</label>
                  <Input value={wsName} onChange={(e) => setWsName(e.target.value)} placeholder="Acme Corp" required autoFocus />
                  {wsName && (
                    <p className="text-xs text-muted-foreground">
                      {wsName.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "workspace"}.dositracker.app
                    </p>
                  )}
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium">Choose a plan</label>
                  <Select value={wsPlan} onChange={(e) => setWsPlan(e.target.value as PlanId)}>
                    <option value="free">Free — $0 · up to 3 seats</option>
                    <option value="starter">Starter — $6/user/mo · 14-day trial</option>
                    <option value="business">Business — $12/user/mo · 14-day trial</option>
                  </Select>
                </div>
                <Button type="submit" size="lg" className="w-full" disabled={!!loading || !wsName.trim()}>
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
                  <label className="text-sm font-medium">Email</label>
                  <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
                </div>
                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <label className="text-sm font-medium">Password</label>
                    <button type="button" className="text-xs text-primary hover:underline">
                      Forgot password?
                    </button>
                  </div>
                  <div className="relative">
                    <Input
                      type={show ? "text" : "password"}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      required
                      className="pr-10"
                    />
                    <button
                      type="button"
                      onClick={() => setShow((s) => !s)}
                      className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    >
                      {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>

                <Button type="submit" size="lg" className="w-full" disabled={!!loading}>
                  {loading && <Loader2 className="h-4 w-4 animate-spin" />}
                  {loading ? "Signing in…" : "Sign in"}
                </Button>
              </form>

              <div className="mt-8">
                <div className="mb-3 flex items-center gap-3">
                  <span className="h-px flex-1 bg-border" />
                  <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    Demo accounts
                  </span>
                  <span className="h-px flex-1 bg-border" />
                </div>

                <div className="stagger space-y-1.5" style={{ ["--stagger-base" as string]: "45ms" }}>
                  {demoAccounts.map((u) => (
                    <button
                      key={u.id}
                      onClick={() => signInAs(u.id)}
                      disabled={!!loading}
                      className="group flex w-full items-center gap-3 rounded-xl border border-border bg-card p-2.5 text-left transition-colors hover:border-primary/35 hover:bg-accent/40 disabled:opacity-60"
                    >
                      <Avatar name={u.name} size="sm" status={u.status} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm font-medium">{u.name}</span>
                          <Badge tone={roleTone[u.role]} className="capitalize">{roleLabels[u.role]}</Badge>
                        </div>
                        <div className="truncate text-xs text-muted-foreground">{roleDescriptions[u.role]}</div>
                      </div>
                      {loading === u.id ? (
                        <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary" />
                      ) : (
                        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
                      )}
                    </button>
                  ))}
                </div>

                <p className="mt-3 text-center text-xs text-muted-foreground">
                  Pick any account to explore its role-specific dashboard. Demo mode — no real password needed.
                </p>

                <button
                  onClick={signInAsHost}
                  disabled={!!loading}
                  className="group mt-4 flex w-full items-center gap-3 rounded-xl border border-primary/25 bg-accent/40 p-2.5 text-left transition-colors hover:border-primary/45 hover:bg-accent/70 disabled:opacity-60"
                >
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg brand-gradient text-white">
                    <Server className="h-[18px] w-[18px]" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium">Platform Admin</span>
                      <Badge tone="primary">Host</Badge>
                    </div>
                    <div className="truncate text-xs text-muted-foreground">Manage all tenants, plans & revenue</div>
                  </div>
                  {loading === "__host__" ? (
                    <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary" />
                  ) : (
                    <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
                  )}
                </button>

                <div className="mt-6 rounded-xl border border-dashed border-border p-3 text-center">
                  <p className="text-sm text-muted-foreground">New to Dosi-Tracker?</p>
                  <button
                    onClick={() => setMode("signup")}
                    className="mt-1 text-sm font-semibold text-primary hover:underline"
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
