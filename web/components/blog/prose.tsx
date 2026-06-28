import { CopyButton } from "@/components/copy-button";

/**
 * Prose primitives for rendering a blog post in the Parler design system.
 * Server components (CodeBlock embeds the client CopyButton). The look matches
 * the landing page: pure-black canvas, graphite hairlines, electric-blue accents.
 */

export function ArticleH2({ id, children }: { id?: string; children: React.ReactNode }) {
  return (
    <h2
      id={id}
      className="scroll-mt-24 mt-16 text-[28px] font-semibold leading-[1.15] tracking-[-0.02em] text-pure-white"
    >
      {children}
    </h2>
  );
}

export function ArticleH3({ id, children }: { id?: string; children: React.ReactNode }) {
  return (
    <h3 id={id} className="scroll-mt-24 mt-10 text-[19px] font-semibold text-frost">
      {children}
    </h3>
  );
}

export function P({ children }: { children: React.ReactNode }) {
  return <p className="mt-5 text-[16px] leading-[1.75] text-mist">{children}</p>;
}

export function Lead({ children }: { children: React.ReactNode }) {
  return <p className="mt-6 text-[18px] leading-[1.7] text-fog">{children}</p>;
}

export function UL({ children }: { children: React.ReactNode }) {
  return <ul className="mt-5 space-y-2.5 text-[16px] leading-[1.7] text-mist">{children}</ul>;
}

export function LI({ children }: { children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="mt-[10px] size-1.5 shrink-0 rounded-full bg-electric-blue" />
      <span>{children}</span>
    </li>
  );
}

export function Em({ children }: { children: React.ReactNode }) {
  return <em className="text-frost not-italic font-medium">{children}</em>;
}

export function InlineCode({ children }: { children: React.ReactNode }) {
  return (
    <code className="rounded-[6px] border border-graphite-rail bg-void-black px-1.5 py-0.5 font-mono text-[13.5px] text-clicked-lavender">
      {children}
    </code>
  );
}

export function Divider() {
  return <hr className="mt-16 border-0 border-t border-graphite-rail" />;
}

const COMMENT_MARKERS = ["//", "#", "--"];

/** Is this whole line a comment? (leading // , # , or -- ) */
function isCommentLine(line: string): boolean {
  const t = line.trimStart();
  return COMMENT_MARKERS.some((m) => t.startsWith(m));
}

/** Split a line into (code, trailing-comment) if a " // " / " -- " / " # " marker exists. */
function splitTrailingComment(line: string): [string, string] {
  let best = -1;
  for (const m of COMMENT_MARKERS) {
    const idx = line.indexOf(` ${m}`);
    if (idx >= 0 && (best === -1 || idx < best)) best = idx;
  }
  return best === -1 ? [line, ""] : [line.slice(0, best), line.slice(best)];
}

function CodeLine({ text }: { text: string }) {
  if (text === "") return <span> </span>;
  if (isCommentLine(text)) return <span className="text-steel">{text}</span>;
  const [code, comment] = splitTrailingComment(text);
  return (
    <>
      <span className="text-frost">{code}</span>
      {comment && <span className="text-steel">{comment}</span>}
    </>
  );
}

/** A code panel with a title bar (dots + label) and a copy button, matching the landing page. */
export function CodeBlock({
  code,
  label,
  lang,
}: {
  code: string;
  label?: string;
  lang?: string;
}) {
  const title = label ?? lang ?? "code";
  return (
    <div className="mt-6 overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black">
      <div className="flex items-center gap-2 border-b border-graphite-rail px-4 py-2.5">
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="ml-2 font-mono text-[12px] text-electric-blue">{title}</span>
        <CopyButton value={code} className="ml-auto" />
      </div>
      <pre className="overflow-x-auto p-5 font-mono text-[13px] leading-[1.7]">
        <code>
          {code.split("\n").map((line, i) => (
            <span key={i}>
              <CodeLine text={line} />
              {"\n"}
            </span>
          ))}
        </code>
      </pre>
    </div>
  );
}

/** A figure: an image with a centered caption. Diagrams and screenshots both use this. */
export function Figure({
  src,
  alt,
  caption,
}: {
  src: string;
  alt: string;
  caption?: string;
}) {
  return (
    <figure className="mt-8">
      <div className="overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={src} alt={alt} className="w-full" loading="lazy" />
      </div>
      {caption && (
        <figcaption className="mt-3 text-center text-[13px] leading-relaxed text-steel">
          {caption}
        </figcaption>
      )}
    </figure>
  );
}

/** A bordered aside for an important note (e.g. the security gate). */
export function Callout({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <div className="mt-8 rounded-[16px] border border-graphite-rail surface-lift p-6">
      {title && <div className="text-[14px] font-semibold text-frost">{title}</div>}
      <div className="text-[15px] leading-[1.7] text-fog [&>p]:mt-0">{children}</div>
    </div>
  );
}

/** A simple two-column reference table styled in the design system. */
export function RefTable({
  head,
  rows,
}: {
  head: [string, string];
  rows: [React.ReactNode, React.ReactNode][];
}) {
  return (
    <div className="mt-6 overflow-hidden rounded-[16px] border border-graphite-rail">
      <table className="w-full border-collapse text-left text-[14px]">
        <thead>
          <tr className="border-b border-graphite-rail bg-void-black">
            <th className="px-5 py-3 font-medium text-frost">{head[0]}</th>
            <th className="px-5 py-3 font-medium text-frost">{head[1]}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} className="border-b border-graphite-rail/60 last:border-0">
              <td className="px-5 py-3.5 align-top text-mist">{r[0]}</td>
              <td className="px-5 py-3.5 align-top text-fog">{r[1]}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
