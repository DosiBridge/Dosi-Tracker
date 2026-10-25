import type { Metadata } from "next";
import { headers } from "next/headers";
import { Inter, Geist_Mono } from "next/font/google";
import "./globals.css";
import { ThemeProvider } from "@/components/theme-provider";
import { SessionProvider } from "@/components/session-provider";
import { RUNTIME_CONFIG_ELEMENT_ID, resolveRuntimeConfig, serializeRuntimeConfig } from "@/lib/runtime-config";
import { NONCE_HEADER } from "@/lib/security-headers";

const inter = Inter({
  variable: "--font-inter",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "Dosi-Tracker — Team Activity & Productivity",
  description:
    "Dosi-Tracker: monitor team activity, time, and productivity across projects with a modern, privacy-conscious dashboard.",
};

const themeScript = `
(function() {
  try {
    var t = localStorage.getItem('dosi-theme');
    if (!t) t = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    if (t === 'dark') document.documentElement.classList.add('dark');
  } catch (e) {}
})();
`;

/**
 * Every page renders per request: the Content-Security-Policy nonce (src/proxy.ts)
 * is new each time, and the deployment's configuration (API address, download
 * location, demo switch) is read from the environment now, not frozen at build.
 */
export default async function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  const nonce = (await headers()).get(NONCE_HEADER) ?? undefined;
  const config = resolveRuntimeConfig(process.env);

  return (
    <html lang="en" suppressHydrationWarning className={`${inter.variable} ${geistMono.variable} h-full antialiased`}>
      <head>
        {/* Data, not code: never executed, so the CSP needs no allowance for it. */}
        <script
          id={RUNTIME_CONFIG_ELEMENT_ID}
          type="application/json"
          dangerouslySetInnerHTML={{ __html: serializeRuntimeConfig(config) }}
        />
        <script nonce={nonce} dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body className="min-h-full font-sans">
        <ThemeProvider>
          <SessionProvider>{children}</SessionProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
