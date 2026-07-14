"use client";

import { useState } from "react";
import { User, ShieldCheck, Palette, Bell, Check, Monitor, Moon, Sun } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { useTheme } from "@/components/theme-provider";
import { useSession } from "@/components/session-provider";
import { roleLabels } from "@/lib/roles";
import { cn } from "@/lib/utils";

const allTabs = [
  { key: "profile", label: "Profile", icon: User, roles: ["owner", "admin", "worker", "client"] },
  { key: "tracking", label: "Tracking", icon: ShieldCheck, roles: ["owner", "admin"] },
  { key: "appearance", label: "Appearance", icon: Palette, roles: ["owner", "admin", "worker", "client"] },
  { key: "notifications", label: "Notifications", icon: Bell, roles: ["owner", "admin", "worker", "client"] },
] as const;

type TabKey = (typeof allTabs)[number]["key"];

export default function SettingsPage() {
  const { user: currentUser } = useSession();
  const tabs = allTabs.filter((t) => (t.roles as readonly string[]).includes(currentUser.role));
  const [tab, setTab] = useState<TabKey>("profile");
  const [saved, setSaved] = useState(false);

  const [prefs, setPrefs] = useState({
    screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true, idleDetection: true, blurScreenshots: false,
  });
  const [notif, setNotif] = useState({ dailyReport: true, weeklyReport: true, lowActivity: false, newMember: true, mentions: true });

  function save() {
    setSaved(true);
    setTimeout(() => setSaved(false), 1600);
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">Manage your profile, tracking, and preferences.</p>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[220px_1fr]">
        {/* Tabs */}
        <nav className="flex gap-1 overflow-x-auto lg:flex-col">
          {tabs.map((t) => {
            const Icon = t.icon;
            return (
              <button
                key={t.key}
                onClick={() => setTab(t.key)}
                className={cn(
                  "flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-sm font-medium transition-colors",
                  tab === t.key ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground"
                )}
              >
                <Icon className="h-[18px] w-[18px]" /> {t.label}
              </button>
            );
          })}
        </nav>

        <div className="space-y-6">
          {tab === "profile" && (
            <Card>
              <CardHeader><CardTitle>Profile</CardTitle></CardHeader>
              <CardContent className="space-y-5">
                <div className="flex items-center gap-4">
                  <Avatar name={currentUser.name} size="lg" status={currentUser.status} />
                  <div>
                    <Button variant="outline" size="sm">Change avatar</Button>
                    <p className="mt-1 text-xs text-muted-foreground">JPG or PNG, up to 2MB.</p>
                  </div>
                </div>
                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                  <Field label="Full name" defaultValue={currentUser.name} />
                  <Field label="Email" defaultValue={currentUser.email} />
                  <Field label="Designation" defaultValue={currentUser.designation} />
                  <div className="space-y-1.5">
                    <label className="text-sm font-medium">Timezone</label>
                    <Select defaultValue={currentUser.timezone}>
                      <option value="Asia/Dhaka">Asia/Dhaka (GMT+6)</option>
                      <option value="Europe/Madrid">Europe/Madrid</option>
                      <option value="America/Los_Angeles">America/Los Angeles</option>
                    </Select>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-sm text-muted-foreground">Role</span>
                  <Badge tone="primary">{roleLabels[currentUser.role]}</Badge>
                </div>
              </CardContent>
            </Card>
          )}

          {tab === "tracking" && (
            <Card>
              <CardHeader><CardTitle>Default tracking preferences</CardTitle></CardHeader>
              <CardContent className="space-y-4">
                <p className="text-sm text-muted-foreground">
                  Defaults for new projects. Each project can override these.
                </p>
                {[
                  ["screenshot", "Screenshots", "Capture periodic screen images"],
                  ["webcam", "Webcam", "Capture webcam snapshots"],
                  ["keyboard", "Keyboard activity", "Count keystrokes (never content)"],
                  ["mouse", "Mouse activity", "Count clicks and movement"],
                  ["activeWindow", "Active window", "Track focused app and title"],
                  ["runningPrograms", "Running programs", "List open applications"],
                  ["idleDetection", "Idle detection", "Pause tracking when inactive"],
                  ["blurScreenshots", "Blur screenshots", "Protect sensitive content on capture"],
                ].map(([key, label, desc]) => (
                  <div key={key} className="border-b border-border pb-4 last:border-0 last:pb-0">
                    <Switch
                      checked={prefs[key as keyof typeof prefs]}
                      onChange={(v) => setPrefs((p) => ({ ...p, [key]: v }))}
                      label={label}
                      description={desc}
                    />
                  </div>
                ))}
              </CardContent>
            </Card>
          )}

          {tab === "appearance" && (
            <Card>
              <CardHeader><CardTitle>Appearance</CardTitle></CardHeader>
              <CardContent className="space-y-4">
                <p className="text-sm text-muted-foreground">Choose how Dosi-Tracker looks to you.</p>
                <ThemePicker />
              </CardContent>
            </Card>
          )}

          {tab === "notifications" && (
            <Card>
              <CardHeader><CardTitle>Notifications</CardTitle></CardHeader>
              <CardContent className="space-y-4">
                {[
                  ["dailyReport", "Daily summary", "Get a daily digest of team activity"],
                  ["weeklyReport", "Weekly report", "Receive weekly productivity reports"],
                  ["lowActivity", "Low activity alerts", "Notify when a member is inactive"],
                  ["newMember", "New members", "When someone joins a project"],
                  ["mentions", "Mentions", "When you're mentioned in a note"],
                ].map(([key, label, desc]) => (
                  <div key={key} className="border-b border-border pb-4 last:border-0 last:pb-0">
                    <Switch
                      checked={notif[key as keyof typeof notif]}
                      onChange={(v) => setNotif((n) => ({ ...n, [key]: v }))}
                      label={label}
                      description={desc}
                    />
                  </div>
                ))}
              </CardContent>
            </Card>
          )}

          <div className="flex items-center justify-end gap-3">
            {saved && (
              <span className="flex items-center gap-1.5 text-sm text-success">
                <Check className="h-4 w-4" /> Saved
              </span>
            )}
            <Button onClick={save}>Save changes</Button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({ label, defaultValue }: { label: string; defaultValue: string }) {
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium">{label}</label>
      <Input defaultValue={defaultValue} />
    </div>
  );
}

function ThemePicker() {
  const { theme, setTheme } = useTheme();
  const options = [
    { key: "light" as const, label: "Light", icon: Sun },
    { key: "dark" as const, label: "Dark", icon: Moon },
    { key: "system" as const, label: "System", icon: Monitor },
  ];

  return (
    <div className="grid grid-cols-3 gap-3">
      {options.map((o) => {
        const Icon = o.icon;
        const activeChoice =
          o.key === "system" ? false : theme === o.key;
        return (
          <button
            key={o.key}
            onClick={() => {
              if (o.key === "system") {
                const sys = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
                setTheme(sys);
              } else setTheme(o.key);
            }}
            className={cn(
              "flex flex-col items-center gap-2 rounded-xl border p-4 transition-colors",
              activeChoice ? "border-primary bg-primary/5" : "border-border hover:bg-muted/60"
            )}
          >
            <Icon className="h-6 w-6" />
            <span className="text-sm font-medium">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
