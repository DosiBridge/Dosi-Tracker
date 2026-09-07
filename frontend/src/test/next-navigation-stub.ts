// Singleton stub for `next/navigation` in jsdom tests.
//
// Usage in a test file (the factory import keeps vi.mock hoisting-safe):
//
//   vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
//   import { __setPathname, __router } from "@/test/next-navigation-stub";
//
// `resetPrototypeState()` in the harness calls `__resetNavigation()` so state
// never leaks between tests.
import { vi } from "vitest";

let pathname = "/dashboard";
let searchParams = new URLSearchParams();

export const __router = {
  push: vi.fn((href: string) => {
    pathname = href.split("?")[0];
    searchParams = new URLSearchParams(href.split("?")[1] ?? "");
  }),
  replace: vi.fn((href: string) => {
    pathname = href.split("?")[0];
    searchParams = new URLSearchParams(href.split("?")[1] ?? "");
  }),
  back: vi.fn(),
  forward: vi.fn(),
  refresh: vi.fn(),
  prefetch: vi.fn(),
};

export function __setPathname(p: string): void {
  pathname = p.split("?")[0];
  searchParams = new URLSearchParams(p.split("?")[1] ?? "");
}

export function __resetNavigation(): void {
  pathname = "/dashboard";
  searchParams = new URLSearchParams();
  __router.push.mockClear();
  __router.replace.mockClear();
  __router.back.mockClear();
  __router.refresh.mockClear();
  __router.prefetch.mockClear();
}

export const usePathname = () => pathname;
export const useRouter = () => __router;
export const useSearchParams = () => searchParams;
export const useParams = () => ({});
export const redirect = vi.fn();
export const notFound = vi.fn();
