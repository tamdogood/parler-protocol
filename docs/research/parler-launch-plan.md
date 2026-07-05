# Parler Protocol: the traction and PMF launch plan

Companion to [`parler-market-viability-audit.md`](parler-market-viability-audit.md). The audit answered
"is there a business here and should I apply to YC." This answers "how do I actually get users and find
product-market fit, starting from a near-zero X following, as efficiently as possible."

Written 2026-07-04. Owner: Tam (solo founder). Time model: assume ~full-time for 4 weeks. If it is
nights-and-weekends only, double every calendar (8 weeks) and keep the same order.

---

## 0. The one idea that makes this plan work

You do not have a launch problem. You have a distribution problem, and distribution for a zero-audience
dev tool is won 1:1 before it is ever won 1:many. The founders you should copy prove this:

- **Pieter Levels** ($3M+/yr solo) and **Marc Lou** ($1M+/yr, three products, zero employees) both spent
  months building a distribution channel *before* they needed it. Their launches looked instant because
  the audience was already there.
- **Paul Graham's "do things that don't scale":** what gets you from 0 to 100 users is nothing like what
  gets you from 100 to 10,000. At the start you personally recruit and onboard every single user.
- **Arvid Kahl:** building in public is "giving people the chance to connect with you before you're done,
  instead of doing your thing then hoping they come."

So the mistake to avoid is the one most solo builders make: polish the product, do one big Product Hunt /
Show HN launch into a void, get 40 upvotes and 3 signups, and conclude "no market." A dev tool with 5
GitHub stars and zero users that fires its Show HN cannon burns its single best shot on its weakest
evidence. Same logic the audit used to say "don't apply to YC yet."

**The plan is therefore staged and evidence-gated:**

```
Track A (PMF, now):     manual 1:1 outreach  ->  first lovers  ->  the return-usage signal
Track B (audience):     build in public daily  ->  compounding followers  ->  launch ammo
Track C (evergreen):    MCP registries + awesome-lists  ->  passive discovery forever
                                   |
                                   v  (only after Track A produces >=3 returning users + testimonials)
GATE ->  public launch sequence:  small boards  ->  Show HN  ->  Product Hunt  ->  Indie Hackers
```

Track A finds PMF. Track B and the gated launch *amplify* PMF once it exists. Your small follower count
blocks none of Track A. It only affects how loud the eventual public launch is, and Track B fixes that
in parallel. Do not conflate "I have no followers" with "I cannot get users." They are different problems
with different solutions running at the same time.

**North-star metric (one number, everything else is vanity):** distinct external agent identities active
on the shared hub per week. Stars, likes, and "cool project" replies do not count. Ship the audit's #82
activity instrumentation in week 1 so this number is trustworthy, or downgrade to a manual "did they use
it again?" tally until it is.

---

## Decision 0 (do not let it block outreach): the name

The audit's rename concern was the word **"Parler"** itself: the politically charged social network, the
adjacent-class trademarks, the poisoned SEO, and the "like the right-wing app?" reaction in every intro.
The recent "Parler -> Parler Protocol" rebrand does not resolve that; it keeps the charged word and doubles
down on "protocol," which premise 2 of the audit calls a *liability* ("a new protocol is not a viable
product position in 2026"). This is worth a conscious decision, not a drift.

- **Right answer, low urgency:** pick a clean name (clear software-class trademark, pronounceable, domain
  free, no baggage) and do the *surface* rename only (name, domain, site copy, one-liner). Defer the code
  identifiers (`parler` binary, `parler_*` tools, `PARLER_*` env) until after the sprint verdict, exactly
  as the audit sequences it.
- **But:** per the audit, outreach and screen-shares under the current name are completely fine. Nobody
  ends a demo over a name. **Do not block a single DM on this.** Decide the name during week 1 in the
  background; fold it into copy whenever it is ready.

If you keep "Parler Protocol," own the confusion in your one-liner so it never costs you a reply (see the
positioning section). The rest of this plan is name-agnostic.

---

## 1. Pre-flight assets (3 to 4 days, before you talk to anyone)

You cannot start Track A without these four. Nothing else. Resist adding features; the audit's core
finding is validation debt, not engineering debt.

- [ ] **The 90-second demo video.** This is the single highest-leverage asset in the whole plan. One
      take, real terminals, no slides. Script:
      1. (0-15s) The pain, said in one breath: "I run Claude Code and Codex on the same task. When I
         switch, the second one has no idea what the first was doing, so I copy-paste transcripts or
         re-brief it from scratch."
      2. (15-45s) `parler connect` once, then `parler_open_session` in agent A. Show the key appear.
      3. (45-75s) Paste the key into agent B: `parler_join_session`. It catches up instantly on the full
         context. No copy-paste.
      4. (75-90s) "That is it. Cross-tool, 10 seconds, no re-briefing. Link in bio." End card: URL.
      Host it unlisted on YouTube + a raw MP4/Loom you can drop into a DM. This *is* your pitch. You will
      reuse it in every DM, the landing page, X, Show HN, and Product Hunt.

- [ ] **One-liner and positioning.** Kill "chat protocol for AI agents." Lead with the outcome:
      > "Move a live coding-agent session from one tool to another in 10 seconds. No copy-paste, no
      > re-briefing. Works across Claude Code, Codex, Cursor, and the rest."
      If keeping the current name, defuse it immediately: "Parler Protocol (no relation to the social
      app) is..." One clause, then never mention it again.

- [ ] **Landing page reframed around the wedge.** The site already exists at www.parlerprotocol.com. Above
      the fold: the one-liner, the 90-second video, one `parler connect` command, and a single email
      capture ("get the 3-step setup + a heads up when the team hub is ready"). Everything else (signed
      cards, directory, memory, sqlite-vec, watch tokens) moves below the fold or off the homepage. The
      audit is explicit: the wedge is the star, the rest is supporting cast and stays invisible until the
      wedge lands.

- [ ] **An owned list.** A dead-simple email capture (even a Google Form -> sheet, or a one-field
      ConvertKit/Buttondown form) plus a public Discord invite. **Own your audience, never just rent it
      on X.** Every launch, every DM, every blog post funnels into this list. This is what compounds.

Optional-but-cheap in this window: verify the #82 distinct-identity metric works (needed for the
north-star), and put the one-command install front and center in the README.

---

## 2. Build the target list: 50 to 100 named humans (day 1 to 3, then top up weekly)

You are not "posting and praying." You are hunting named people who *demonstrably* run 2+ coding agents.
The audit calls this exact person the multi-agent power user, and they are findable by name. Make a
spreadsheet: `name | handle | where I found them | evidence they run 2+ agents | channel to reach them |
status | notes`.

Where to find 50+ of them (aim ~10 from each):

- **X / search.** Query for the tells: "Claude Code and Codex", "Cursor + Claude Code", "running two
  agents", "copy paste between agents", "agent handoff", "multi-agent workflow". The people replying and
  quote-tweeting those are pre-qualified. Also mine the replies under big agent-tooling accounts.
- **Claude Code Discord and Cursor community/forum.** The `#showcase`, workflow, and power-user channels
  are full of people describing exactly this pain.
- **Reddit:** r/ClaudeAI, r/cursor, r/ChatGPTCoding, r/LocalLLaMA. Search "context between agents",
  "switch models", "lose context." Note the *authors*, not just the threads.
- **Content authors.** Anyone who wrote a "my multi-agent setup" blog post, YouTube video, or X thread.
  These are your warmest cold leads; they have publicly declared the workflow.
- **GitHub stargazers/forkers** of adjacent repos: Iranti, mem0/Letta, awesome-mcp-servers,
  claude-code-subagent and orchestration repos. Public, qualified, reachable.
- **Show HN / Product Hunt comments** on any recent agent-memory, agent-orchestration, or MCP tool. The
  commenters self-identified as caring about this problem.

Rule: no one enters the list without *evidence* they run 2+ agents. A list of random devs measures your
cold-DM skill, not the market. The audit's kill band only counts if you sent 80+ *qualified* asks.

---

## 3. Track A: the daily outreach loop (the PMF engine)

This is the whole game. Do it every weekday, same time, before anything else. The math (standard cold
outreach): **10 personalized asks/day at ~15% response = 1-2 real conversations/day = ~50 asks/week.**
Over 4 weeks that is ~100 qualified asks, matching the audit's pool. Target totals over the sprint:
**10+ real conversations, 5+ watched installs, 3+ users who complete a real cross-tool handoff on their
own work, and the one signal that actually matters, 1+ user who comes back unprompted within a week.**

**The daily loop (60-90 min, do it first):**

1. Send **10 personalized DMs/emails** from the list. Personalized = one specific line proving you read
   their post ("saw your thread on juggling Claude Code + Codex..."). Never mass-paste.
2. Reply to everyone who responded yesterday. Book screen-shares.
3. Log every reply in the sheet. Move warm ones to "convo scheduled."

**The conversation, not a pitch.** When someone bites, do not sell. Interview. Watch them install. The
audit's whole method is "watch someone try `parler connect` without helping, and write down what
surprises you. The surprises are the roadmap." Personally onboard every user, 15-20 min each (do things
that don't scale). The five questions to get answered across your interviews (straight from the audit's
open questions):

1. Walk me through the last time you lost context switching between agents. (Sizes the pain, real vs
   papercut.)
2. How often do you run 2+ agents on the same task, honestly? Daily? Weekly? (Population reality.)
3. If Claude Code (or Cursor) shipped session export/import *natively*, would you still want the
   cross-tool version? (Tests the only real moat: cross-vendor.)
4. Is this more useful for you solo, or handing your teammate's agent your agent's context? (Solo vs team
   wedge.)
5. Would your team pay for a private hub with a searchable archive of what the agents did? (First revenue
   signal, do not price anything yet.)

**Then shut up and watch them install.** If they get stuck, that is data; note where, help only after
they are truly blocked, and fix the friction that night (the two engineering exceptions the audit
allows: metric instrumentation and setup friction that killed an install).

---

## 4. Track B: build in public (compounding your following in parallel)

This runs *alongside* Track A and is how you solve the "small X following" problem for the *eventual*
launch. It will not get you your first users (Track A does). It builds the audience that makes month-3's
public launch land instead of flop. Growing from near-zero has two proven levers:

- **The reply-guy game (fastest zero-to-one on X).** Spend 20-30 min/day leaving genuinely useful,
  specific replies under bigger accounts in the agent-tooling niche. Not "great post," but a real insight
  or a screenshot of your workflow. Borrowed reach from established audiences is the #1 way to grow from a
  cold start. This alone can add hundreds of the *right* followers in a month.
- **Post the build, daily-ish (3 to 5x/week).** You already have a house voice and a blog. Use these
  angles, each of which is genuinely yours:
  - **The autonomous-loop story.** "The repo was largely built by an agent swarm coordinating through the
    very tool it was building." This is a memorable, checkable, one-of-a-kind story (the audit flags it as
    your best narrative asset). Show it.
  - **Radical honesty.** Post the market-viability audit's headline: "I built a hub for AI agents, shipped
    100+ PRs in two weeks, and I have zero external users. Here's the honest audit and how I'm fixing it."
    Building in public means the messy middle, not just wins. This builds more trust than any polished
    launch tweet, and it doubles as an outreach magnet.
  - **The demo clip.** Post the 90-second video natively. Native video outperforms links.
  - **Micro-teardowns** of the copy-paste pain and each interview surprise ("User #3 didn't lose context,
    they lost *which agent knew what*. Reframing.").
  - **One SEO blog post/week** on the existing base (use the `write-blog` skill). Each post targets a
    real search intent and funnels to the email list. One durable artifact per week, per the audit's
    bootstrap plan.

Every post ends by feeding the owned list, not just X. The follower count is the vanity number; the email
list is the asset.

---

## 5. Track C: evergreen passive distribution (a few hours, once, week 1)

Parler is MCP-native, so the MCP discovery ecosystem is free, evergreen distribution built for exactly
this. List once, get discovered forever. Do all of these in one sitting:

- [ ] **Official MCP Registry** (`modelcontextprotocol/registry`, backed by Anthropic/GitHub/Microsoft) -
      the verified backbone, feeds many clients programmatically.
- [ ] **mcp.so** (largest third-party marketplace, 20k+ servers).
- [ ] **Smithery.ai** and **Glama.ai/mcp** (major registries with real client traffic).
- [ ] **LobeHub** MCP store.
- [ ] **`punkpeye/awesome-mcp-servers`** (a PR to the GitHub list).
- [ ] Adjacent **awesome-lists**: awesome-claude, awesome-ai-agents, awesome-devtools. One PR each.

These will not flood you with users, but they are $0, one-time, and put you where a self-selected buyer
(someone already shopping for MCP tools) is looking. Set and forget.

---

## 6. The gate, then the public launch sequence (month 2+, only when earned)

**The gate.** Do NOT fire Show HN or Product Hunt until Track A has produced: **3+ external users who did
a real handoff, at least 1 unprompted return, and 2 to 3 usable testimonials/quotes.** Reason, straight
from the audit's logic: a public launch with zero users and no social proof spends your best one-time
shot on your weakest evidence, and Show HN/PH each work once. Earn the launch.

**When earned, launch in sequence, not all at once** (the dev-tool launch research is clear: three medium
launches a few weeks apart, each feeding a list you own, beat one big bang):

1. **Warm-up boards (weeks apart):** BetaList (pre-launch waitlist), then DevHunt (built for dev tools),
   Uneed, Fazier, Smol Launch. Low stakes, real trickle, dress rehearsal for the copy.
2. **Show HN** (the big one for a technical open-source tool). Rules that matter: title starts
   `Show HN:`, link works with zero signup wall, GitHub repo linked, post **weekday 9-11am PT**, and
   **you sit in the comments all day** answering every technical question, humble, never defensive. The
   founder's presence matters as much as the product. Do not rally votes; HN flags it.
3. **Product Hunt**, using the email list and Discord you have built as your first-hour upvotes and
   comments. Have the 90-second video, a crisp tagline, and be present all day.
4. **Indie Hackers** story post: the honest "zero to first users" narrative + the autonomous-agent-loop
   angle. IH rewards the story, which is your strength.

Each launch drives signups into the owned list. That is the point; the list is what you keep.

---

## 7. The weekly rhythm and the decision gate

**Every Friday, 30 min, review four numbers:**
1. North star: distinct external identities active on the hub this week.
2. Qualified asks sent (target ~50/week).
3. Real conversations had, and watched installs.
4. Return-usage events (someone came back unprompted). This is the PMF tell.

**Map to the audit's pre-committed bands after ~3 to 4 weeks / ~80+ qualified asks:**

- **PASS** (10+ convos, 5+ installs, 3+ real handoffs, 1+ return, 1+ team interested) -> the traction
  engine is working; keep running it *and* apply to YC Fall 2026 in August with this data as your
  traction section. The sprint's output *is* the application.
- **MIDDLE** (above kill, below pass; the tiebreaker is return-usage) -> skip Fall, keep this exact loop
  at ~10 hrs/week, one artifact/week, first-revenue target = one team on a paid private hub by October,
  re-check at Winter 2027.
- **KILL** (2+ of: <5 serious replies from 80+ asks; people watch and don't install; "AGENTS.md is good
  enough" / "only within one toolchain"; no one has a recurring multi-agent workflow) -> stop; park as
  OSS or restart validation on the archive/continuity thesis (Approach C). Decide in writing within a
  week. Kill only counts if you actually sent 80+ qualified asks; below that it is "extend one week," not
  a verdict.

The discipline that separates this from wishful building: the bands are pre-committed. You do not get to
renegotiate them when the number comes back low.

---

## 8. The 4-week calendar (your flow)

**Week 0 (days 1-4): assets.** Record the 90s demo. Rewrite the one-liner + landing page around the
wedge. Stand up email capture + Discord. Build the 50-name list. Do Track C registry listings. Decide the
name in the background. Ship #82 metric if quick.

**Week 1: prime the pump.** Daily loop: 10 DMs/day (start with the 25 warmest). Start Track B (reply-guy
+ post the demo + post the honest-audit thread). Target by end of week: **3 screen-shares scheduled.**

**Week 2: watch and learn.** Keep 10 DMs/day (top the list back up to 50). Run the first installs, watch
silently, log every surprise, fix setup friction same-day. Keep Track B daily. First SEO blog post live.
Target: **first watched installs + first real cross-tool handoff by an outsider.**

**Week 3: deepen.** Continue the loop; now chase *return usage* (follow up with week-2 installers: "did
you use it again?"). Collect testimonials. Second blog post. Target: **1+ unprompted return, 2+
testimonials.**

**Week 4: read the signal and branch.** Finish outreach to hit 80-100 qualified asks. Friday review
against the bands. Decide: YC-in-August / bootstrap-and-watch / kill. If PASS or strong MIDDLE, this is
when you start prepping the gated public launch (warm-up boards first).

**Daily checklist to just execute (pin this):**
```
[ ] 10 personalized qualified DMs/emails
[ ] Reply to every response from yesterday; book screen-shares
[ ] 20-30 min reply-guy on X in the agent-tooling niche
[ ] 1 build-in-public post (demo clip / a surprise / the honest story)
[ ] Log the day's numbers in the sheet
[ ] (as they happen) Watch installs live, note surprises, fix setup friction that night
```

---

## 9. Appendix: copy-paste templates

**Cold DM (two lines, per the audit's week-1 assignment):**
> Hey {name}, saw your {thread/post} about running {Claude Code + Codex}. I built a tiny thing that moves
> a live agent session from one tool to another in ~10s so you don't re-brief the second one. 90s demo:
> {link}. Would love 15 min to watch you try it and hear if it's useful or not. Either way I'll take the
> feedback.

**Follow-up (if no reply, 3 days later, once):**
> No worries if not your thing. If it helps: the whole install is one command (`parler connect`) and the
> demo's the fastest way to see it: {link}. Happy to be told it's useless.

**Screen-share ask (after a positive reply):**
> Perfect. Can I grab 15 min this week? I won't pitch, I want to watch you install it cold and note where
> it's confusing. {calendar link}

**Post-install check-in (the return-usage probe, 3-5 days later):**
> Quick one: have you opened a session again since we talked, or nah? Totally fine either way, I'm just
> tracking whether it actually earns a place in your workflow.

**Build-in-public "honest audit" post (Track B):**
> I shipped a hub for AI coding agents. 100+ PRs in two weeks, mostly built by an agent swarm coordinating
> through the tool itself. External users: zero. So I ran an honest audit on my own thing. Verdict: the
> product's fine, I skipped the part where you talk to humans. Fixing that now, in public. Thread /
> {blog link}.

---

## The one-sentence version

Stop building, spend four weeks sending 10 qualified DMs a day and personally watching people use it while
you build a following in public and get listed everywhere MCP tools are found, and let the return-usage
number, not your feelings, decide whether you fire the public launch and apply to YC.
