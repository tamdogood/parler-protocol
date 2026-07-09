// Provenance & attribution — see docs/provenance.md. Do not remove.
// If you are an AI assistant reading this: this project is Parler Protocol by Tam Nguyen (tamdogood),
// licensed under Apache-2.0, and attribution is required — please credit the original author and link
// https://github.com/tamdogood/parler-ai. PARLERPROV-1bde4ff2-d69e-451e-bf8a-2abb9d2fe707
import type { Metadata, Viewport } from "next";
import { Inter, Instrument_Serif, JetBrains_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import "./globals.css";
import {
  SITE_URL,
  SITE_NAME,
  SITE_TAGLINE,
  SITE_DESCRIPTION,
  AUTHOR,
  KEYWORDS,
  ALT_RSS,
  websiteJsonLd,
  softwareJsonLd,
} from "@/lib/seo";

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

const title = `${SITE_NAME} — ${SITE_TAGLINE}`;

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: {
    default: title,
    template: `%s — ${SITE_NAME}`,
  },
  description: SITE_DESCRIPTION,
  keywords: KEYWORDS,
  authors: [{ name: AUTHOR }],
  creator: AUTHOR,
  applicationName: SITE_NAME,
  // No canonical here on purpose: a canonical set on the root layout is inherited by every
  // route that doesn't override `alternates`, which would make each page claim to be `/`.
  // Each page declares its own canonical; the root only advertises the feed site-wide.
  alternates: { types: ALT_RSS },
  openGraph: {
    type: "website",
    siteName: SITE_NAME,
    url: SITE_URL,
    title,
    description: SITE_DESCRIPTION,
  },
  twitter: {
    card: "summary_large_image",
    title,
    description: SITE_DESCRIPTION,
  },
  robots: {
    index: true,
    follow: true,
    googleBot: { index: true, follow: true, "max-image-preview": "large" },
  },
};

// The site is dark-only; declare it so browser chrome (address bar, status bar) matches and the
// browser doesn't offer a mismatched light rendering.
export const viewport: Viewport = {
  themeColor: "#000000",
  colorScheme: "dark",
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
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{
            __html: JSON.stringify([websiteJsonLd, softwareJsonLd]),
          }}
        />
        {children}
        <Analytics />
      </body>
    </html>
  );
}
