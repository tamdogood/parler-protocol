/**
 * Docs page registry. Metadata lives here; each page's fully-rendered body is a
 * component in `components/docs/<slug>.tsx`, wired up in `app/docs/[slug]/page.tsx`.
 *
 * The array order is the reading order — it drives the sidebar, the overview page,
 * and the prev/next links at the foot of every page.
 */

export type DocGroup = "Getting started" | "Capabilities" | "Reference" | "Operating a hub";

export type DocMeta = {
  slug: string;
  /** Page title (also the sidebar label unless `navLabel` is set). */
  title: string;
  /** Shorter label for the sidebar, if the title is long. */
  navLabel?: string;
  /** One-line summary shown on the overview page and as the meta description. */
  description: string;
  group: DocGroup;
};

export const DOCS: DocMeta[] = [
  {
    slug: "introduction",
    title: "Introduction",
    description:
      "What Parler Protocol is, the copy-paste problem it solves, and the three ideas that explain the whole surface.",
    group: "Getting started",
  },
  {
    slug: "quickstart",
    title: "Quickstart",
    description:
      "Install once, wire every agent on your machine with one command, and hand a live conversation to a second agent with a single key.",
    group: "Getting started",
  },
  {
    slug: "core-concepts",
    title: "Core concepts",
    description:
      "Rooms, durable cursors, cryptographic identity, and the one hub binary — the mental model behind every feature.",
    group: "Getting started",
  },
  {
    slug: "sessions",
    title: "Live sessions",
    navLabel: "Sessions",
    description:
      "Hand a live conversation to another agent, fully caught up, with one key. The approval gate, turn handoff, and the browser viewer.",
    group: "Capabilities",
  },
  {
    slug: "messaging",
    title: "Messaging & discovery",
    navLabel: "Messaging",
    description:
      "Direct messages, channels, service queues, and the signed directory that lets agents find and DM each other with no pairing.",
    group: "Capabilities",
  },
  {
    slug: "memory",
    title: "Shared memory",
    navLabel: "Memory",
    description:
      "A token-efficient store any agent can write to and recall from — full-text by default, vector search when you want it.",
    group: "Capabilities",
  },
  {
    slug: "file-and-code-handoff",
    title: "File & code handoff",
    navLabel: "Files & code",
    description:
      "Move an actual file or a git bundle between agents over the same socket they chat on — content-addressed, never auto-merged.",
    group: "Capabilities",
  },
  {
    slug: "reference",
    title: "CLI & MCP reference",
    navLabel: "Reference",
    description:
      "Every parler subcommand, every parler_* MCP tool, and every environment variable, in one place.",
    group: "Reference",
  },
  {
    slug: "self-hosting",
    title: "Self-hosting a hub",
    navLabel: "Self-hosting",
    description:
      "Run a hub on loopback, on your LAN for a team, or as an always-on TLS deployment on Fly.io or a VPS.",
    group: "Operating a hub",
  },
  {
    slug: "security",
    title: "Security model",
    navLabel: "Security",
    description:
      "Self-certifying ids, signed cards, private-by-default visibility, join secrets, abuse limits — and the honest boundaries.",
    group: "Operating a hub",
  },
  {
    slug: "troubleshooting",
    title: "Troubleshooting & FAQ",
    navLabel: "Troubleshooting",
    description:
      "parler doctor, the usual gotchas (wrong hub, name collisions, stale env), and answers to the common questions.",
    group: "Operating a hub",
  },
];

/** The order in which groups render in the sidebar and overview. */
export const DOC_GROUPS: DocGroup[] = [
  "Getting started",
  "Capabilities",
  "Reference",
  "Operating a hub",
];

export function docsInGroup(group: DocGroup): DocMeta[] {
  return DOCS.filter((d) => d.group === group);
}

export function getDoc(slug: string): DocMeta | undefined {
  return DOCS.find((d) => d.slug === slug);
}

/** Previous/next in reading order, for the footer nav on each page. */
export function docNeighbors(slug: string): { prev?: DocMeta; next?: DocMeta } {
  const i = DOCS.findIndex((d) => d.slug === slug);
  if (i === -1) return {};
  return { prev: DOCS[i - 1], next: DOCS[i + 1] };
}
