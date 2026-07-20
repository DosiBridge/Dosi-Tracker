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
import { loginApi, registerApi } from "@/hooks/useApi";
import type { Role } from "@/lib/types";

const roleTone: Record<Role, "primary" | "success" | "info" | "warning"> = {
  host: "primary",
  owner: "primary",
  admin: "warning",
  worker: "success",
  client: "info",
};

// Demo mode removed.

export default function LoginPage() {
  const router = useRouter();
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading("signin");
    setErrorMsg(null);
    try {
      await loginApi(email, password);
      // Wait a moment for token to settle, then redirect
      setTimeout(() => router.push("/dashboard"), 500);
    } catch (err: any) {
      setErrorMsg("Invalid username or password");
      setLoading(null);
    }
  }

  async function onCreateWorkspace(e: React.FormEvent) {
    e.preventDefault();
    if (!wsName.trim() || !password) return;
    setLoading("__signup__");
    setErrorMsg(null);
    try {
      await registerApi(email, password, wsName);
      // Automatically log them in after registration
      await loginApi(email, password);
      setTimeout(() => router.push("/dashboard"), 600);
    } catch (err: any) {
      setErrorMsg(err.message || "Registration failed");
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

                <div className="space-y-1.5">
                  <label className="text-sm font-medium">Email</label>
                  <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" required autoFocus />
                </div>
                <div className="space-y-1.5">
                  <label className="text-sm font-medium">Password</label>
                  <Input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={6} />
                </div>
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
