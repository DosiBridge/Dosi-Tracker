import { notFound } from "next/navigation";
import type { ComponentType } from "react";
import { reportCatalog } from "@/lib/reports-data";
import { TimeActivityReport } from "@/components/reports/time-activity-report";
import { ProductivityReport } from "@/components/reports/productivity-report";
import { AppsReport } from "@/components/reports/apps-report";
import { ProjectsReport } from "@/components/reports/projects-report";
import { AttendanceReport } from "@/components/reports/attendance-report";
import { PayrollReport } from "@/components/reports/payroll-report";
import { WeeklyReport } from "@/components/reports/weekly-report";

const map: Record<string, ComponentType> = {
  "time-activity": TimeActivityReport,
  productivity: ProductivityReport,
  "apps-urls": AppsReport,
  projects: ProjectsReport,
  attendance: AttendanceReport,
  payroll: PayrollReport,
  weekly: WeeklyReport,
};

export function generateStaticParams() {
  return reportCatalog.map((r) => ({ slug: r.slug }));
}

export default async function ReportPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const Report = map[slug];
  if (!Report) notFound();
  return <Report />;
}
