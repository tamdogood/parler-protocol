"use client";

import { useId, useState } from "react";
import { ArrowRight, Check, Mail } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  Dialog,
  DialogTrigger,
  DialogModalContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { HUB_API } from "@/lib/api";

type Status = "idle" | "submitting" | "success" | "error";

// A basic client-side sanity check so an obvious typo never round-trips to the hub.
// The server is the source of truth (a 400 still renders the invalid-email message).
const LOOKS_LIKE_EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

const INVALID_MSG = "That email doesn't look right.";
const UNREACHABLE_MSG = "Couldn't reach the hub. Try again in a minute.";

const BLURB =
  "Drop your email and we'll send the 3-step setup, plus a heads-up when the team hub is ready. No spam, no more than an email or two.";

/**
 * The owned list, as a pop-up. A compact trigger card (matching the sibling viewer CTA) opens a
 * centered modal holding one field and one button that POSTs to the hub's `/api/waitlist`
 * (`{ ok: true }` on 200, 400 = bad email, 429 / network = try again later). Lives directly under
 * the sessions wedge so it captures the reader at the moment the payoff lands.
 */
export function EmailCapture() {
  const [open, setOpen] = useState(false);
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState("");
  const inputId = useId();

  const submitting = status === "submitting";
  const done = status === "success";

  // Fresh every time the modal opens: a stale error or "you're on the list" never greets a re-open.
  function resetForm() {
    setEmail("");
    setStatus("idle");
    setError("");
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = email.trim();
    if (submitting || done) return;

    if (!LOOKS_LIKE_EMAIL.test(value)) {
      setStatus("error");
      setError(INVALID_MSG);
      return;
    }

    setStatus("submitting");
    setError("");

    try {
      const res = await fetch(`${HUB_API}/api/waitlist`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: value }),
      });

      if (res.ok) {
        setStatus("success");
        return;
      }

      setStatus("error");
      setError(res.status === 400 ? INVALID_MSG : UNREACHABLE_MSG);
    } catch {
      setStatus("error");
      setError(UNREACHABLE_MSG);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) resetForm();
      }}
    >
      {/* The trigger card — mirrors the "Watch a session" CTA so the two sit as a matched pair. */}
      <div className="flex flex-col gap-4 rounded-[16px] border border-graphite-rail bg-void-black p-6 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
            <Mail className="size-5 text-electric-blue" />
          </span>
          <div>
            <h3 className="text-[16px] font-semibold text-pure-white">Get the 3-step setup</h3>
            <p className="mt-1 max-w-xl text-[14px] leading-relaxed text-fog">{BLURB}</p>
          </div>
        </div>
        <DialogTrigger
          className={buttonVariants({ variant: "cta", size: "default", className: "shrink-0" })}
        >
          <Mail className="size-4" />
          Notify me
        </DialogTrigger>
      </div>

      <DialogModalContent>
        <div>
          <DialogTitle>Get the 3-step setup</DialogTitle>
          <DialogDescription className="mt-2">{BLURB}</DialogDescription>
        </div>

        {done ? (
          <p
            className="flex items-center gap-2 text-[14px] font-medium text-delivered-green"
            role="status"
            aria-live="polite"
          >
            <Check className="size-4" />
            You&apos;re on the list.
          </p>
        ) : (
          <form onSubmit={submit} className="flex flex-col gap-3" noValidate>
            <div>
              <label htmlFor={inputId} className="sr-only">
                Email address
              </label>
              <Input
                id={inputId}
                type="email"
                inputMode="email"
                autoComplete="email"
                placeholder="you@company.com"
                value={email}
                onChange={(e) => {
                  setEmail(e.target.value);
                  if (status === "error") {
                    setStatus("idle");
                    setError("");
                  }
                }}
                disabled={submitting}
                autoFocus
                aria-invalid={status === "error"}
                aria-describedby={error ? `${inputId}-error` : undefined}
              />
            </div>
            <Button type="submit" variant="cta" size="default" disabled={submitting}>
              {submitting ? "Sending…" : "Notify me"}
              {!submitting && <ArrowRight className="size-4" />}
            </Button>

            {/* Errors are announced assertively; the field is re-usable so the reader can fix and resubmit. */}
            {error && (
              <p
                id={`${inputId}-error`}
                className="text-[13px] text-bounced-red"
                role="alert"
                aria-live="assertive"
              >
                {error}
              </p>
            )}
          </form>
        )}
      </DialogModalContent>
    </Dialog>
  );
}
