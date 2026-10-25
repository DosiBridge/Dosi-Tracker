import Link from "next/link";
import { Download } from "lucide-react";
import { CopyButton } from "@/components/dashboard/agent-setup";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { AGENT_PLATFORMS, checksumsUrl, releaseAssetUrl, type AgentPlatform } from "@/lib/agent-downloads";
import { cn } from "@/lib/utils";

/* ============================================================================
 * Building blocks of the public /install guides. Server components (the
 * copy button is the only client piece), fed from lib/agent-downloads so the
 * guides, the dashboard's Download page and the docs name the same files and
 * commands.
 * ========================================================================== */

/** A stable, readable element id from a heading or label. */
function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "");
}

/** "Guides / <platform>" above a guide's title. */
export function GuideBreadcrumb({ current }: { current: string }) {
  return (
    <nav aria-label="Breadcrumb" className="mb-2 flex items-center gap-2 text-xs">
      <Link href="/install" className="text-muted-foreground hover:text-foreground">
        Guides
      </Link>
      <span className="text-muted-foreground/60" aria-hidden="true">
        /
      </span>
      <span aria-current="page" className="font-semibold text-primary-strong">
        {current}
      </span>
    </nav>
  );
}

/** A section of a guide: an h2 and its content. */
export function GuideSection({ title, children }: { title: string; children: React.ReactNode }) {
  const id = `section-${slug(title)}`;
  return (
    <section aria-labelledby={id} className="space-y-3">
      <h2 id={id} className="font-display text-lg font-bold text-foreground">
        {title}
      </h2>
      {children}
    </section>
  );
}

/**
 * Commands to type, one per line, each its own element (so a reader, a
 * screen reader and a test can pick out one), with one button copying them
 * all. `label` names the block for the copy button ("Copy Install the .deb").
 */
export function CommandBlock({ label, commands, className }: { label: string; commands: readonly string[]; className?: string }) {
  const labelId = `cmd-${slug(label)}`;
  return (
    <figure className={cn("overflow-hidden rounded-xl border border-border/80 bg-muted/50", className)}>
      <figcaption className="flex items-center justify-between gap-2 border-b border-border/60 px-3 py-1.5">
        <span id={labelId} className="text-[11px] font-medium text-muted-foreground">
          {label}
        </span>
        <CopyButton value={commands.join("\n")} labelledBy={labelId} />
      </figcaption>
      <pre className="overflow-x-auto px-3 py-2.5 font-mono text-[12px] leading-relaxed text-foreground">
        {commands.map((line) => (
          <code key={line} className="block whitespace-pre">
            {line}
          </code>
        ))}
      </pre>
    </figure>
  );
}

/** The platform's download buttons (recommended first) and the checksums link. */
export function AssetDownloads({ platform, downloadBaseUrl }: { platform: AgentPlatform; downloadBaseUrl: string }) {
  const info = AGENT_PLATFORMS[platform];
  return (
    <div className="space-y-3">
      <ul className="grid grid-cols-1 gap-3 sm:grid-cols-3" aria-label={`${info.name} downloads`}>
        {info.assets.map((asset, i) => (
          <li key={asset.file} className="flex flex-col gap-1.5 rounded-xl border border-border/80 bg-card p-3">
            <a
              href={releaseAssetUrl(downloadBaseUrl, asset.file)}
              className={cn(linkButtonClass(i === 0 ? "primary" : "outline", "md"), "w-full")}
            >
              <Download aria-hidden className="h-4 w-4" /> {asset.label}
            </a>
            <span className="break-all text-center font-mono text-[11px] text-foreground">{asset.file}</span>
            <span className="text-center text-[11px] text-muted-foreground">{asset.note}</span>
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">
        Each release also publishes{" "}
        <a href={checksumsUrl(downloadBaseUrl)} className="font-medium text-primary-strong hover:underline">
          SHA256SUMS
        </a>
        , the SHA-256 checksum of every file: compare it with your download before installing.
      </p>
    </div>
  );
}

/** A two-column "term: meaning" table (permissions, switches, files). */
export function FactTable({
  caption,
  headers,
  rows,
}: {
  caption: string;
  headers: [string, string] | [string, string, string];
  rows: React.ReactNode[][];
}) {
  return (
    <div className="overflow-x-auto rounded-xl border border-border/80 bg-card">
      <table className="w-full text-left text-xs">
        <caption className="sr-only">{caption}</caption>
        <thead className="border-b border-border/80 bg-muted/50 font-semibold text-foreground">
          <tr>
            {headers.map((h) => (
              <th key={h} scope="col" className="p-3">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-border/60 text-muted-foreground">
          {rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) =>
                j === 0 ? (
                  <th key={j} scope="row" className="p-3 align-top font-medium text-foreground">
                    {cell}
                  </th>
                ) : (
                  <td key={j} className="p-3 align-top">
                    {cell}
                  </td>
                ),
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Inline code. */
export function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.92em] text-foreground">{children}</code>;
}
