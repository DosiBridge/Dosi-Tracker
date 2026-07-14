"use client";

import { useState } from "react";
import { Check, ChevronRight, ChevronLeft } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Avatar } from "@/components/ui/avatar";
import { users } from "@/lib/tenant-data";
import type { Project, TrackingPermissions } from "@/lib/types";
import { cn } from "@/lib/utils";

const steps = ["Details", "Team", "Tracking"];
const colors = ["#6d5efc", "#ec4899", "#0ea5e9", "#22c55e", "#f59e0b", "#a855f7"];

const permLabels: { key: keyof TrackingPermissions; label: string; desc: string }[] = [
  { key: "screenshot", label: "Screenshots", desc: "Capture periodic screen images" },
  { key: "webcam", label: "Webcam", desc: "Capture webcam snapshots" },
  { key: "keyboard", label: "Keyboard activity", desc: "Count keystrokes (not content)" },
  { key: "mouse", label: "Mouse activity", desc: "Count clicks & movement" },
  { key: "activeWindow", label: "Active window", desc: "Track focused app & title" },
  { key: "runningPrograms", label: "Running programs", desc: "List open applications" },
];

export function CreateProjectModal({
  open,
  onClose,
  onCreate,
}: {
  open: boolean;
  onClose: () => void;
  onCreate: (p: Project) => void;
}) {
  const [step, setStep] = useState(0);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [color, setColor] = useState(colors[0]);
  const [members, setMembers] = useState<string[]>([]);
  const [interval, setInterval] = useState(10);
  const [perms, setPerms] = useState<TrackingPermissions>({
    screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true,
  });

  function reset() {
    setStep(0); setTitle(""); setDescription(""); setColor(colors[0]); setMembers([]);
    setInterval(10);
    setPerms({ screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true });
  }

  function finish() {
    onCreate({
      id: `p${Date.now()}`,
      title: title || "Untitled Project",
      description: description || "No description",
      color,
      archived: false,
      intervalMinutes: interval,
      permissions: perms,
      memberIds: members,
      createdAt: new Date().toISOString().slice(0, 10),
      loggedThisWeek: 0, loggedThisMonth: 0, loggedTotal: 0,
    });
    reset();
    onClose();
  }

  const canNext = step === 0 ? title.trim().length > 0 : true;

  return (
    <Modal open={open} onClose={onClose} title="Create project" description="Set up tracking for a new project.">
      {/* Stepper */}
      <div className="mb-6 flex items-center">
        {steps.map((s, i) => (
          <div key={s} className="flex flex-1 items-center last:flex-none">
            <div className="flex items-center gap-2">
              <div
                className={cn(
                  "flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold transition-colors",
                  i < step ? "bg-primary text-primary-foreground" : i === step ? "bg-primary/15 text-primary ring-2 ring-primary" : "bg-muted text-muted-foreground"
                )}
              >
                {i < step ? <Check className="h-3.5 w-3.5" /> : i + 1}
              </div>
              <span className={cn("text-xs font-medium", i === step ? "text-foreground" : "text-muted-foreground")}>{s}</span>
            </div>
            {i < steps.length - 1 && <div className={cn("mx-2 h-px flex-1", i < step ? "bg-primary" : "bg-border")} />}
          </div>
        ))}
      </div>

      {step === 0 && (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Project name</label>
            <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Mobile App Revamp" />
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Description</label>
            <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Short summary" />
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Color</label>
            <div className="flex gap-2">
              {colors.map((c) => (
                <button
                  key={c}
                  onClick={() => setColor(c)}
                  className={cn("h-8 w-8 rounded-full transition-transform", color === c && "ring-2 ring-offset-2 ring-offset-card scale-110")}
                  style={{ background: c, boxShadow: color === c ? `0 0 0 2px ${c}` : undefined }}
                />
              ))}
            </div>
          </div>
        </div>
      )}

      {step === 1 && (
        <div className="space-y-2">
          <p className="text-sm text-muted-foreground">Select team members to add to this project.</p>
          <div className="max-h-72 space-y-1 overflow-y-auto">
            {users.filter((u) => u.role !== "client").map((u) => {
              const selected = members.includes(u.id);
              return (
                <button
                  key={u.id}
                  onClick={() => setMembers((m) => (selected ? m.filter((x) => x !== u.id) : [...m, u.id]))}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg border p-2.5 text-left transition-colors",
                    selected ? "border-primary bg-primary/5" : "border-border hover:bg-muted/60"
                  )}
                >
                  <Avatar name={u.name} size="sm" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">{u.name}</div>
                    <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                  </div>
                  <div className={cn("flex h-5 w-5 items-center justify-center rounded-full border", selected ? "border-primary bg-primary text-primary-foreground" : "border-border")}>
                    {selected && <Check className="h-3 w-3" />}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Snapshot interval</label>
            <div className="flex gap-2">
              {[5, 10, 15, 20, 30].map((v) => (
                <button
                  key={v}
                  onClick={() => setInterval(v)}
                  className={cn(
                    "flex-1 rounded-lg border py-2 text-sm font-medium transition-colors",
                    interval === v ? "border-primary bg-primary/10 text-primary" : "border-border hover:bg-muted/60"
                  )}
                >
                  {v}m
                </button>
              ))}
            </div>
          </div>
          <div className="space-y-3 rounded-lg border border-border p-3">
            {permLabels.map((p) => (
              <Switch
                key={p.key}
                checked={perms[p.key]}
                onChange={(v) => setPerms((prev) => ({ ...prev, [p.key]: v }))}
                label={p.label}
                description={p.desc}
              />
            ))}
          </div>
        </div>
      )}

      {/* Footer */}
      <div className="mt-6 flex items-center justify-between">
        <Button variant="ghost" onClick={step === 0 ? onClose : () => setStep((s) => s - 1)}>
          {step === 0 ? "Cancel" : (<><ChevronLeft className="h-4 w-4" /> Back</>)}
        </Button>
        {step < steps.length - 1 ? (
          <Button onClick={() => setStep((s) => s + 1)} disabled={!canNext}>
            Continue <ChevronRight className="h-4 w-4" />
          </Button>
        ) : (
          <Button onClick={finish}>
            <Check className="h-4 w-4" /> Create project
          </Button>
        )}
      </div>
    </Modal>
  );
}
