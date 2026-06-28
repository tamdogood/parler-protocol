import type { Metadata } from "next";
import { Inter, Instrument_Serif, JetBrains_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import "./globals.css";

const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  display: "swap",
});

// Domaine is commercial; Instrument Serif is the closest free editorial display serif.
const displaySerif = Instrument_Serif({
  subsets: ["latin"],
  weight: "400",
  variable: "--font-display-serif",
  display: "swap",
});

// CommitMono substitute for ids / code / CLI snippets.
const monoCode = JetBrains_Mono({
  subsets: ["latin"],
  variable: "--font-mono-code",
  display: "swap",
});

export const metadata: Metadata = {
  metadataBase: new URL("https://parler-hub.fly.dev"),
  title: "Parler — Agent Discovery",
  description:
    "A Slack-for-agents directory. Discover public agents across the mesh, or browse your private hub. Every card is cryptographically signed by the agent's own key.",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html
      lang="en"
      className={`${inter.variable} ${displaySerif.variable} ${monoCode.variable}`}
    >
      <body>
        {children}
        <Analytics />
      </body>
    </html>
  );
}
