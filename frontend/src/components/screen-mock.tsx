import type { ScreenMock } from "@/lib/types";
import { cn } from "@/lib/utils";

/**
 * Renders a lightweight, offline-friendly "screenshot" mockup for a captured
 * app instead of shipping real images. Style varies by app kind.
 */
export function ScreenMockView({
  screen,
  title,
  className,
}: {
  screen: ScreenMock;
  title?: string;
  className?: string;
}) {
  return (
    <div className={cn("relative overflow-hidden rounded-lg bg-[#0f1320] text-[10px]", className)}>
      {/* window chrome */}
      <div className="flex items-center gap-1.5 border-b border-white/10 bg-black/30 px-2 py-1.5">
        <span className="h-2 w-2 rounded-full bg-[#ff5f57]" />
        <span className="h-2 w-2 rounded-full bg-[#febc2e]" />
        <span className="h-2 w-2 rounded-full bg-[#28c840]" />
        <span className="ml-2 truncate text-[9px] text-white/50">{title ?? screen.app}</span>
      </div>
      <div className="h-full">{renderBody(screen)}</div>
    </div>
  );
}

function bar(w: string, c: string, h = "h-1.5") {
  return <div className={cn("rounded-sm", h)} style={{ width: w, background: c }} />;
}

function renderBody(screen: ScreenMock) {
  const a = screen.accent;
  switch (screen.kind) {
    case "editor":
      return (
        <div className="flex h-full">
          <div className="flex w-8 flex-col gap-1.5 bg-black/40 p-1.5">
            {[a, "#ffffff30", "#ffffff30", "#ffffff30"].map((c, i) => (
              <div key={i} className="h-2 w-2 rounded" style={{ background: c }} />
            ))}
          </div>
          <div className="flex-1 space-y-1.5 p-2">
            {bar("35%", a)}
            {bar("70%", "#ffffff20")}
            {bar("55%", "#ffffff20")}
            {bar("62%", "#ffffff20")}
            {bar("40%", a + "aa")}
            {bar("72%", "#ffffff20")}
            {bar("48%", "#ffffff20")}
          </div>
        </div>
      );
    case "browser":
      return (
        <div className="space-y-1.5 p-2">
          <div className="flex gap-1">
            <div className="h-3 flex-1 rounded-full bg-white/10" />
          </div>
          <div className="h-10 rounded" style={{ background: `linear-gradient(120deg, ${a}, ${a}55)` }} />
          <div className="grid grid-cols-3 gap-1.5">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-6 rounded bg-white/10" />
            ))}
          </div>
        </div>
      );
    case "design":
      return (
        <div className="flex h-full gap-1.5 p-2">
          <div className="flex-1 rounded" style={{ background: `radial-gradient(circle at 30% 30%, ${a}, ${a}33)` }} />
          <div className="w-8 space-y-1.5">
            {bar("100%", "#ffffff20")}
            {bar("80%", "#ffffff20")}
            {bar("90%", a)}
            {bar("60%", "#ffffff20")}
          </div>
        </div>
      );
    case "chat":
      return (
        <div className="space-y-1.5 p-2">
          {[["55%", false], ["70%", true], ["40%", false], ["62%", true], ["48%", false]].map(
            ([w, mine], i) => (
              <div key={i} className={cn("flex", mine ? "justify-end" : "justify-start")}>
                <div className="h-3 rounded-full" style={{ width: w as string, background: mine ? a : "#ffffff18" }} />
              </div>
            )
          )}
        </div>
      );
    case "terminal":
      return (
        <div className="space-y-1 p-2 font-mono">
          {["$ cargo run --release", "  Compiling dosi-tracker", "  Finished release", "$ ▉"].map((t, i) => (
            <div key={i} className="truncate" style={{ color: i === 0 || i === 3 ? a : "#ffffff55" }}>
              {t}
            </div>
          ))}
        </div>
      );
    case "docs":
    default:
      return (
        <div className="space-y-1.5 p-2">
          {bar("50%", a)}
          {bar("92%", "#ffffff18")}
          {bar("85%", "#ffffff18")}
          {bar("70%", "#ffffff18")}
          {bar("88%", "#ffffff18")}
        </div>
      );
  }
}
