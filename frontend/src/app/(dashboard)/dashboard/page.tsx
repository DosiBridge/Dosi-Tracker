"use client";

import { useSession } from "@/components/session-provider";
import { AdminDashboard } from "@/components/dashboard/admin-dashboard";
import { WorkerDashboard } from "@/components/dashboard/worker-dashboard";
import { ClientDashboard } from "@/components/dashboard/client-dashboard";
import { SetupChecklist } from "@/components/dashboard/setup-checklist";

export default function DashboardPage() {
  const { user } = useSession();

  if (user.role === "owner" || user.role === "admin") {
    const isOwner = user.role === "owner";
    return <AdminDashboard userName={user.name.split(" ")[0]} isOwner={isOwner} onboarding={<SetupChecklist isOwner={isOwner} />} />;
  }
  if (user.role === "client") {
    return <ClientDashboard user={user} />;
  }
  return <WorkerDashboard user={user} />;
}
