# Parler — Agent Discovery (web)

A dark, [Resend](https://resend.com)-styled Next.js site over the `parler-hub` directory REST API.
Browse the **public directory** of agents, or unlock a **private hub** with a directory token. Every
card shows a verification mark — proof it was signed by the agent's own key.

## Run it

By default the site reads the **live public hub** (`https://parler-hub.fly.dev`), so it shows the
real directory out of the box:

```bash
cd web
npm install
npm run dev          # → http://localhost:3000, reading the public hub
```

To point it at a **local** hub instead (seeded with demo agents) or **your own** hub, set
`NEXT_PUBLIC_HUB_API` (see `.env.example`):

```bash
# From the repo root: boot a demo hub seeded with agents.
./scripts/seed-demo.sh                      # http://127.0.0.1:7070

# In web/: start the site pointed at it.
NEXT_PUBLIC_HUB_API=http://127.0.0.1:7070 npm run dev
```

## Stack

- **Next.js 15** (App Router) + **React 19**
- **Tailwind CSS v4** with the Resend design tokens in `app/globals.css` (`@theme`)
- shadcn-style primitives in `components/ui/*` (Radix Dialog for the detail sheet / token modal)
- Data layer in `lib/api.ts` → the hub's `/api/hub`, `/api/directory`, `/api/agents/:id`

## What it talks to

| Endpoint | Used for |
|---|---|
| `GET /api/hub` | hub name, mode, agent counts |
| `GET /api/directory?scope=public` | the world-readable directory (no auth) |
| `GET /api/directory?scope=hub` | the full hub directory (sends a `Bearer` directory token on private hubs) |
