/**
 * A static, server-rendered FAQ block that also emits FAQPage structured data.
 *
 * Unlike components/faq.tsx (the site FAQ, a client-side accordion), this one is
 * parameterized and renders every answer as visible prose. That keeps the JSON-LD in
 * lockstep with what a crawler sees, which is the condition for FAQ rich results. Used by
 * the SEO landing pages (/agent-protocol, /agent-communication).
 */

export type SeoFaqItem = { q: string; a: string };

export function SeoFaq({
  eyebrow,
  heading,
  items,
}: {
  eyebrow: string;
  heading: string;
  items: SeoFaqItem[];
}) {
  const jsonLd = {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: items.map((it) => ({
      "@type": "Question",
      name: it.q,
      acceptedAnswer: { "@type": "Answer", text: it.a },
    })),
  };

  return (
    <section className="border-t border-graphite-rail">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />
      <div className="mx-auto max-w-[760px] px-6 py-16">
        <p className="text-[14px] font-medium text-electric-blue">{eyebrow}</p>
        <h2 className="mt-3 text-[28px] font-semibold leading-[1.15] tracking-[-0.02em] text-pure-white">
          {heading}
        </h2>
        <dl className="mt-8">
          {items.map((it) => (
            <div
              key={it.q}
              className="border-t border-graphite-rail py-6 first:border-0 first:pt-0"
            >
              <dt className="text-[17px] font-semibold text-frost">{it.q}</dt>
              <dd className="mt-2.5 text-[16px] leading-[1.75] text-mist">{it.a}</dd>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}
