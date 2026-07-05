"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Download, Link2, Share2 } from "lucide-react";
import type { SessionMessage, SessionView } from "@/lib/types";
import { buildWrapped, fmtCompact, fmtDuration } from "@/lib/wrapped";
import { CARD_H, CARD_W, drawWrapped } from "@/lib/wrapped-canvas";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Canonical origin for share links when the page hasn't mounted yet (SSR / first paint). */
const SITE_ORIGIN = "https://www.parlerprotocol.com";

function shareUrlFor(token?: string): string {
  const origin = typeof window !== "undefined" ? window.location.origin : SITE_ORIGIN;
  return token ? `${origin}/wrapped#k=${encodeURIComponent(token)}` : `${origin}/wrapped`;
}

/**
 * The shareable "Session Wrapped" scorecard: a canvas-rendered card (so what you see is what
 * downloads, pixel-for-pixel) plus the share rail — download the PNG for Instagram/Facebook, share
 * the image straight to the OS share sheet on mobile, copy a link, or post to X/Facebook. Reused by
 * the in-viewer modal and the standalone `/wrapped` page.
 */
export function WrappedShare({
  view,
  messages,
  token,
  className,
}: {
  view: SessionView;
  messages?: SessionMessage[];
  token?: string;
  className?: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapped = useMemo(() => buildWrapped(view, messages), [view, messages]);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);

  const shareUrl = useMemo(() => shareUrlFor(token), [token]);
  const shareText = useMemo(
    () =>
      `${wrapped.agentCount} agents · ≈${fmtCompact(wrapped.totalTokens)} tokens · ${fmtDuration(
        wrapped.durationMs,
      )} — my agents' Session Wrapped, built on Parler Protocol.`,
    [wrapped],
  );

  // Paint once fonts are ready so the exported text uses Inter when the page has loaded it.
  useEffect(() => {
    let cancelled = false;
    const paint = () => {
      if (!cancelled && canvasRef.current) drawWrapped(canvasRef.current, wrapped);
    };
    const fonts = typeof document !== "undefined" ? document.fonts : undefined;
    if (fonts?.ready) fonts.ready.then(paint).catch(paint);
    else paint();
    return () => {
      cancelled = true;
    };
  }, [wrapped]);

  const toBlob = () =>
    new Promise<Blob | null>((resolve) => {
      const c = canvasRef.current;
      if (!c) return resolve(null);
      c.toBlob(resolve, "image/png");
    });

  async function download() {
    const blob = await toBlob();
    if (!blob) return;
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `session-wrapped-${wrapped.roomLabel.replace(/[^a-z0-9]+/gi, "-") || "parler"}.png`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    setSaved(true);
    setTimeout(() => setSaved(false), 1600);
  }

  async function shareNative() {
    const blob = await toBlob();
    const file = blob ? new File([blob], "session-wrapped.png", { type: "image/png" }) : null;
    // Prefer sharing the image itself (opens Instagram/Facebook/etc. in the OS share sheet on mobile).
    if (file && navigator.canShare?.({ files: [file] })) {
      try {
        await navigator.share({ files: [file], title: "Session Wrapped", text: shareText });
        return;
      } catch {
        /* user cancelled or share failed — fall through */
      }
    }
    if (typeof navigator.share === "function") {
      try {
        await navigator.share({ title: "Session Wrapped", text: shareText, url: shareUrl });
        return;
      } catch {
        /* fall through to copy */
      }
    }
    copyLink();
  }

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(shareUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard blocked */
    }
  }

  const openX = () =>
    window.open(
      `https://twitter.com/intent/tweet?text=${encodeURIComponent(shareText)}&url=${encodeURIComponent(shareUrl)}`,
      "_blank",
      "noopener,noreferrer",
    );
  const openFacebook = () =>
    window.open(
      `https://www.facebook.com/sharer/sharer.php?u=${encodeURIComponent(shareUrl)}`,
      "_blank",
      "noopener,noreferrer",
    );

  return (
    <div className={cn("flex flex-col items-center gap-6", className)}>
      {/* The card. A soft bloom sits behind it; the canvas itself downloads pixel-for-pixel. */}
      <div className="relative w-full max-w-[360px]">
        <div className="pointer-events-none absolute -inset-5 rounded-[36px] bg-[radial-gradient(60%_50%_at_50%_0%,rgba(59,158,255,0.28),transparent_70%),radial-gradient(50%_40%_at_80%_10%,rgba(146,129,247,0.22),transparent_70%)] blur-2xl" />
        <canvas
          ref={canvasRef}
          width={CARD_W}
          height={CARD_H}
          className="relative h-auto w-full rounded-[24px] border border-graphite-rail shadow-[0_30px_80px_-30px_rgba(59,158,255,0.5)]"
          aria-label="Session Wrapped scorecard"
        />
      </div>

      {/* Share rail. */}
      <div className="flex w-full max-w-[360px] flex-col gap-3">
        <Button variant="cta" size="lg" className="w-full" onClick={download}>
          {saved ? <Check className="size-4" /> : <Download className="size-4" />}
          {saved ? "Saved to your device" : "Download image"}
        </Button>

        <div className="grid grid-cols-2 gap-2">
          <Button variant="outline" onClick={shareNative}>
            <Share2 className="size-4" />
            Share
          </Button>
          <Button variant="outline" onClick={copyLink}>
            {copied ? <Check className="size-4 text-delivered-green" /> : <Link2 className="size-4" />}
            {copied ? "Link copied" : "Copy link"}
          </Button>
          <Button variant="outline" onClick={openX}>
            Post to X
          </Button>
          <Button variant="outline" onClick={openFacebook}>
            Facebook
          </Button>
        </div>

        <p className="text-center text-[12px] leading-relaxed text-steel">
          Save the card and post it to your Instagram story or feed — or drop the link anywhere. The
          image is a snapshot; no watch code is baked in.
        </p>
      </div>
    </div>
  );
}
