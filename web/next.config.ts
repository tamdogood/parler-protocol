import type { NextConfig } from "next";

// Content-Security-Policy for a fully static / prerendered marketing site.
//
// We intentionally allow 'unsafe-inline' for scripts and styles: Next injects inline
// bootstrap/hydration scripts and Tailwind emits inline styles, and the only nonce-based
// alternative forces per-request dynamic rendering — which would throw away the edge-cached
// static prerender this whole site relies on. We still lock down the vectors that matter
// (object-src, base-uri, frame-ancestors) and pin the exact origins we actually talk to:
//   - self:                      the app + self-hosted next/font files + Vercel Analytics beacons
//   - parler-hub.fly.dev:        the live hub REST API (directory + sessions fetch from it)
//   - va.vercel-scripts.com:     the Vercel Analytics script
const contentSecurityPolicy = [
  "default-src 'self'",
  "script-src 'self' 'unsafe-inline' https://va.vercel-scripts.com",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: https:",
  "font-src 'self'",
  "connect-src 'self' https://parler-hub.fly.dev https://va.vercel-scripts.com",
  "frame-ancestors 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "object-src 'none'",
  "upgrade-insecure-requests",
].join("; ");

const securityHeaders = [
  { key: "Content-Security-Policy", value: contentSecurityPolicy },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
];

const nextConfig: NextConfig = {
  reactStrictMode: true,
  // Ship only the icons/primitives each page actually uses instead of the barrel bundle.
  experimental: {
    optimizePackageImports: ["lucide-react", "@radix-ui/react-dialog"],
  },
  async headers() {
    return [{ source: "/:path*", headers: securityHeaders }];
  },
};

export default nextConfig;
