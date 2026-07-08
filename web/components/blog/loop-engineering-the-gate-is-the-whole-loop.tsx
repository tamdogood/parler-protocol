import { ArticleH2, P, Lead, Em, A, InlineCode, CodeBlock, UL, LI, Callout, RefTable } from "@/components/blog/prose";

/** The fully-rendered body of "Loop engineering: the gate is the whole loop." */
export function LoopEngineeringTheGateIsTheWholeLoop() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Loop engineering is the skill that replaced prompt engineering this year.{" "}
        <A href="https://www.mindstudio.ai/blog/what-is-loop-engineering-ai-coding-agents">Addy Osmani</A>{" "}
        named it in June, synthesizing what Boris Cherny and a few others had been doing: you stop typing
        prompts by hand and start designing the cycle an agent runs on its own. Find the work, do it, check
        it, remember what happened, repeat, with no human re-prompting each turn.
      </Lead>
      <P>
        Every guide I have read since then spends its length on the prompt. The reasoning template, the
        persona, the step list. I built a chunk of this repo with an autonomous loop, and I think that
        focus is backwards. The prompt is the least important part. The gate is the whole thing, and almost
        nobody writes about it, because a good gate is boring and a clever prompt demos well.
      </P>

      <ArticleH2 id="prompt-to-loop">Loop engineering moved the skill from the prompt to the loop</ArticleH2>
      <P>
        The canonical loop is six verbs: plan, search, modify, verify, repair, summarize. The agent acts,
        observes the result, reasons about it, and goes again until a goal is met. That is the part the
        explainers get right, and it is a real shift. A one-shot agent generates code once and hopes. A
        looped agent runs the code, reads the error, and the error becomes the context for the next turn.
        Coding was always iterative. The loop just lets the agent iterate without you sitting there.
      </P>
      <P>
        But a loop is four parts, not one, and the guides collapse three of them into the prompt. Here is
        how they split in this repo:
      </P>
      <UL>
        <LI>
          <Em>The prompt</Em> answers &quot;what should the agent do this turn.&quot; It lives in{" "}
          <InlineCode>.claude/commands/work-next.md</InlineCode>.
        </LI>
        <LI>
          <Em>The tools</Em> answer &quot;what can it do.&quot; Those are Claude Code&apos;s file, bash, and
          git tools. I wrote none of them.
        </LI>
        <LI>
          <Em>The feedback</Em> answers &quot;did it work.&quot; That is{" "}
          <InlineCode>scripts/verify.sh</InlineCode>, one command that exits pass or fail.
        </LI>
        <LI>
          <Em>The guardrails</Em> answer &quot;when must it stop.&quot; Those are a handful of rules that
          keep a bad turn from becoming a bad afternoon.
        </LI>
      </UL>
      <P>
        Three of those four are not the prompt. And the one that decides whether the whole thing is
        trustworthy is the feedback.
      </P>

      <ArticleH2 id="prompt-least-important">The prompt is the least important part</ArticleH2>
      <P>
        My per-iteration prompt is about sixty lines and deliberately dull. It says: read the lessons file,
        take the top unchecked item off the backlog, state it in a sentence, split it if it is too big to
        land behind one green gate, make the smallest change that finishes it, run the gate, commit only if
        the gate passed, then stop. One item per turn. That is the whole reasoning template.
      </P>
      <P>
        There is no persona, no &quot;you are a 10x engineer,&quot; no chain-of-thought incantation. The
        prompt does not need to be clever because the clever part is not the agent talking itself through a
        plan. It is the gate telling the agent, in one deterministic signal, whether the plan worked. A weak
        prompt with a strong gate self-corrects. A strong prompt with a weak gate produces confident,
        plausible, broken code and checks its own homework.
      </P>
      <P>
        That is the sentence that reorganized how I think about this: a loop with nothing to push back is
        the agent agreeing with itself on repeat.
      </P>

      <ArticleH2 id="the-gate">The gate is a single command that cannot lie</ArticleH2>
      <P>
        So the highest-leverage file in the loop is not the prompt. It is{" "}
        <InlineCode>verify.sh</InlineCode>. Its only job is to answer &quot;am I done, or did I break
        something,&quot; fast and the same way every time. The contract is two lines of output the loop greps
        for:
      </P>
      <CodeBlock
        label="scripts/verify.sh (the gated stages)"
        lang="bash"
        code={`# the contract: exit non-zero and print VERIFY: FAIL (<stage>) on any failure
fail() { echo "VERIFY: FAIL ($1)"; exit 1; }

cargo build  --workspace --all-targets --locked                || fail "rust-build"
cargo clippy --workspace --all-targets --locked -- -D warnings || fail "clippy"
cargo test   --workspace --locked                              || fail "rust-test"

echo "VERIFY: PASS"`}
      />
      <P>
        Two properties make this a signal the agent can trust instead of a suggestion.
      </P>
      <P>
        It is deterministic. Same tree in, same verdict out. <InlineCode>--locked</InlineCode> pins the
        dependency versions so a background crate update cannot flip the result between two runs.{" "}
        <InlineCode>-D warnings</InlineCode> means a clippy nag is a hard failure, not a shrug, so the agent
        cannot pass by leaving a mess the linter would catch. The gate has no opinions and no mood.
      </P>
      <P>
        And it mirrors CI exactly. The comment at the top of the script is a promise: green locally equals
        green in CI, no surprises after a push. If the two ever drift, the gate is lying to the agent, and a
        lying gate is worse than no gate, because the agent trusts it. Keeping <InlineCode>verify.sh</InlineCode>{" "}
        and <InlineCode>.github/workflows/ci.yml</InlineCode> identical is not housekeeping. It is the thing
        that makes the feedback real.
      </P>
      <P>
        The agent runs this before it touches anything, to confirm the tree is green on a clean checkout, and
        again after every change. A failure is not a dead end. It is the input to the repair turn: read the
        stage that failed, read the output, fix the root cause, run it again.
      </P>

      <ArticleH2 id="push-back">A loop with nothing to push back agrees with itself</ArticleH2>
      <P>
        The gate only pushes back if there is something to push against. That is why the prompt makes the
        agent add a test that would fail without its change, before it calls the item done. A green gate on
        code with no test proving the new behavior is not evidence. It is the absence of evidence dressed as
        a pass.
      </P>
      <P>
        This is the part that separates a loop that ships from a loop that drifts. The agent is good at
        producing code that looks right. It is not good at knowing when it is wrong, for the same reason it
        wrote the wrong thing in the first place. The test is the second opinion the model cannot talk out
        of. The gate runs it. The agent does not get to grade itself.
      </P>

      <ArticleH2 id="guardrails">The guardrails are what make it safe to walk away</ArticleH2>
      <P>
        A closed loop with a good gate will still run off a cliff if nothing tells it to stop. The guardrails
        are short and they are the difference between leaving it running and babysitting it.
      </P>
      <P>
        The most important one is the no-progress guard. If the same gate failure survives two fix attempts,
        the agent stops trying:
      </P>
      <Callout title="No-progress guard (from work-next.md)">
        <p>
          If the same failure survives two fix attempts, stop, write{" "}
          <InlineCode>[BLOCKED] &lt;stage&gt;: &lt;what you saw&gt;</InlineCode> next to the item in the
          backlog, append the finding to the lessons file, and report. Do not thrash.
        </p>
      </Callout>
      <P>
        Without that, a stuck agent will burn an hour and a lot of tokens making the same wrong fix in
        slightly different words. Two strikes and it parks the item and moves on. The rest of the guardrails
        are in the same spirit. One item per turn, so a bad turn is one commit to revert, not a tangled
        branch. Never commit a red tree. Never relax <InlineCode>-D warnings</InlineCode> to make the gate
        pass, which is the agent&apos;s most tempting shortcut and its most dangerous one. Additive changes
        only, because the hub is deployed live and a breaking wire change needs a human.
      </P>
      <P>
        State lives in git. Each finished item is its own commit and its checkbox in the backlog, so if the
        loop dies mid-run, the next start reads the checkboxes and resumes exactly where it left off. Crash
        recovery is free when your state is commits.
      </P>

      <ArticleH2 id="what-it-cannot-do">What the loop cannot do</ArticleH2>
      <P>
        I would rather name the edges than imply there are none, because a loop that hides them is the one
        that wastes your weekend.
      </P>
      <P>
        It cannot feed itself. The loop is only as good as the backlog, and a human writes the backlog. Each
        item has to be about the size of one pull request with an explicit &quot;done when&quot; line. Hand
        it a vague epic and it either thrashes or does something small and technically-correct that misses
        the point. Writing good small items is the real work that moved upstream when the coding moved to the
        agent.
      </P>
      <P>
        It does not touch the website. The <InlineCode>web/</InlineCode> app is human-driven and out of the
        loop&apos;s scope on purpose. A backlog item that needs UI work does only its Rust and protocol half
        and leaves a <InlineCode>[HUMAN] web:</InlineCode> note. Some judgment I did not want to automate, so
        I did not.
      </P>
      <P>
        And it does not decide when something is truly done, only when the gate is green. Green means it
        builds, lints clean, and the tests pass. It does not mean the design is right. That is why the prompt
        tells the agent to stop and surface anything that feels architecturally wrong instead of pushing
        through. The gate catches broken. It does not catch wrong. A human still owns wrong.
      </P>

      <ArticleH2 id="read-the-gate">Read the gate, then run one turn</ArticleH2>
      <P>
        None of this is a framework. It is four files: a backlog of small items, a sixty-line prompt, a gate
        script that mirrors CI, and a lessons file the agent reads at the top of every turn. The whole
        operating manual is in{" "}
        <A href="https://github.com/tamdogood/parler-ai/blob/main/docs/loop-engineering.md">
          docs/loop-engineering.md
        </A>
        , and the gate itself is{" "}
        <A href="https://github.com/tamdogood/parler-ai/blob/main/scripts/verify.sh">scripts/verify.sh</A>,
        under sixty lines you can read in a minute.
      </P>
      <P>
        If you want to see what this loop actually shipped, the messy version is in{" "}
        <A href="/blog/bugs-that-hid-until-production">the bugs that hid until production</A>, and the
        architecture it was building is <A href="/blog/stop-copy-pasting-between-ai-agents">the deep dive</A>.
        Then go look at your own agent setup and ask the only question that matters: after it makes a change,
        what one command tells it whether it worked, and would you bet a push on that command being right? If
        you do not have that command, you do not have a loop. You have an agent guessing with extra steps.
      </P>
      <RefTable
        head={["The four parts of a loop", "Where it lives here"]}
        rows={[
          ["Prompt: what to do this turn", "A dull 60-line work-next.md, one item per turn"],
          ["Tools: what it can do", "Claude Code's file, bash, and git tools"],
          ["Feedback: did it work", "verify.sh: one deterministic command that mirrors CI"],
          ["Guardrails: when to stop", "No-progress guard, additive-only, git-backed resume"],
        ]}
      />
    </article>
  );
}
