# Dross Autonomy Audit

Date: 2026-03-18

This is a transcript-first audit of recent Dross behavior, focused on the failure modes Nate actually felt: misinterpretation, handholding, unnecessary process overhead, tool confusion, and low autonomy.

The point is not "the model was bad." The point is that the system around the model is manufacturing confusion, then feeding that confusion back into memory, recall, and operator experience.

## Evidence Base

Reviewed recent session logs under `~/.dross/sessions/` plus relevant Hermes runtime code in `~/Desktop/Hermes/`.

Representative sessions:

- `session_20260311_070815_70fb99.json` — simple commit-review ask with heavy discovery/search overhead before first answer.
- `session_20260310_113253_86d20b.json` — JEPA/PDF request where Dross insists file delivery tools are missing, then later finds plumbing mismatch.
- `session_20260310_114628_4087c871.json` and `20260310_114628_4087c871.jsonl` — same work, but the JSON snapshot shows repeated turns that do not appear in the raw JSONL transcript.
- `session_20260310_143543_ce1cfe.json` — asked to save a markdown file locally, but the active tool surface exposed only `read_file`.
- `session_20260310_104234_db502a.json` — large patch batches causing partial-apply failures and operator frustration.
- `session_20260310_154932_fb144a.json` — environment/venv mismatch during PDF tooling work.

Representative Hermes code:

- `run_agent.py`
- `agent/prompt_builder.py`
- `toolsets.py`
- `model_tools.py`
- `tools/session_search_tool.py`
- `gateway/session.py`

## Direct Answer

Dross feels "confused and braindead" less because the base model is incapable and more because the runtime keeps putting it in self-conflicting states:

1. the stored conversation state is getting polluted
2. the prompt tells it capabilities exist that are not actually exposed
3. discovery/planning tools are framed as default behavior instead of exception paths
4. memory is being treated like a diary instead of a curated knowledge layer
5. artifact delivery semantics are split across prompt hints, tool availability, and platform behavior instead of one authoritative contract

That combination produces exactly the behaviors Nate disliked: fake uncertainty, handholding, repeated re-orientation, tool indecision, and low forward momentum.

## Findings

### 1. Transcript state is being duplicated and then fed back into the agent

This is the most important structural bug.

Observed behavior:

- `session_20260310_114628_4087c871.json` repeats the same user asks and assistant responses three times.
- The raw transcript file `20260310_114628_4087c871.jsonl` does not show those duplicate user turns.
- That means the duplication is not primarily "Nate repeated himself." It is a persistence/view corruption problem.

Likely root cause in Hermes:

- `run_agent.py:_log_msg_to_db()` appends messages to SQLite immediately.
- `run_agent.py:_flush_messages_to_session_db()` later appends the same turn again from the in-memory `messages` list.
- Future turns load history from the polluted store, so the agent sees duplicated prior context.

Why this matters:

- Session recall quality drops.
- Memory extraction gets noisier.
- The model starts from a distorted conversation state.
- The operator experiences "why are you repeating yourself / why do you think I said this three times?"

This is not a cosmetic logging issue. It directly degrades cognition.

### 2. Prompt reality and runtime reality are out of sync

Observed behavior:

- In `session_20260310_143543_ce1cfe.json`, Dross says it cannot save the file locally because only `read_file` is available. The session tool surface indeed only had one tool: `read_file`.
- In other sessions, Dross has prompt hints that say CLI artifact generation should use `render_pdf` plus `send_file`, even when those tools are not actually in the visible tool list for that turn.

Relevant code:

- `agent/prompt_builder.py` hardcodes platform hints like "prefer render_pdf plus send_file".
- `toolsets.py` claims `_DROSS_CORE_TOOLS` includes a very broad set of tools.
- `run_agent.py` actually exposes tools through mode/capability routing, which can result in a much smaller live surface.

Effect:

- The model is told "you can do X" by the prompt while the live tool surface says "no you cannot."
- It then oscillates between confidence and apology.
- Nate experiences this as confusion or low competence, but the runtime itself is creating the contradiction.

The rule should be:

- never mention a tool in the prompt unless that exact tool is visible right now

### 3. Discovery and planning scaffolding are firing far too early

Observed behavior:

- In `session_20260311_070815_70fb99.json`, a simple "what do you think of the last commits?" question incurred 22 pre-visible tool calls before the first real answer.
- The tool chain included `discover_capabilities`, `todo`, `search_files`, `session_search`, and multiple terminal reads before answering a straightforward repo question.

Relevant code:

- `agent/prompt_builder.py:170-180` says to use `discover_capabilities` before specialized tools.
- `agent/prompt_builder.py:81-85` pushes proactive `session_search`.
- `toolsets.py` includes `todo`, `session_search`, `discover_capabilities`, `memory`, and delegation in the default core toolset.

Effect:

- Dross pays an autonomy tax before doing obvious work.
- Simple asks get escalated into mini-orchestration rituals.
- The operator sees a system that "needs help getting started" instead of one that just acts.

These tools should be recovery or escalation surfaces, not default first moves.

### 4. Memory behavior is too eager, too noisy, and not trustworthy enough

Observed behavior:

- The system prompt literally encourages saving memory "like a diary."
- Sessions write many transient implementation facts into memory.
- Some memory entries shown in tool output are stale, noisy, or contradictory.

Relevant code:

- `agent/prompt_builder.py:75-79` tells the model to proactively save important things and use memory like a diary.
- `tools/session_search_tool.py` summarizes past sessions with an auxiliary model, which can smear details when the source store is already noisy.

Effect:

- Memory becomes a dump of ephemeral implementation chatter, not a high-signal operator model.
- Dross becomes more likely to "remember" design notes, stale assumptions, or partial tool states as if they are current runtime truth.
- Nate then gets answers shaped by stale memory instead of current state.

Memory should capture durable truth, preferences, decisions, and verified outcomes. Not every clever intermediate observation.

### 5. Tooling reliability is too brittle for the ambition level

Observed behavior:

- Patch batches partially applied and contaminated files.
- A reinstall script targeted `.venv` while the launcher used `venv`.
- Verification steps themselves hit quoting and environment issues.

Effect:

- Dross burns cycles repairing its own tool path.
- Nate has to coach execution strategy instead of getting leverage.
- The system looks low-autonomy because the tool layer is unreliable enough that "be more autonomous" would just mean "fail faster."

The operator does not care whether the problem lives in the model or the shell wrapper. The user-visible result is the same: wasted turns.

### 6. Delivery semantics are not authoritative enough

Observed behavior:

- CLI sessions, Telegram sessions, and local file output all depend on a mix of prompt instructions, env vars, tool exposure, and post-hoc MEDIA tag behavior.
- Dross often has to reason about delivery mode instead of returning a clean artifact contract.

Effect:

- The agent has to decide too many transport details in-language.
- This increases confusion and creates unnecessary branching.

The model should return an artifact intent. The platform layer should decide how to deliver it.

## What Actually Worked

The system is not uniformly bad. Some capabilities were genuinely strong:

- bounded research asks usually produced useful answers
- once the right PDF/export tools were actually present, Dross could deliver artifacts
- when forced into small, explicit patch increments, execution quality improved materially

So the issue is not "throw everything away." The issue is to stop routing good model/tool ability through a confusing control plane.

## Fixes

### P0: Fix state truth before anything else

1. Make one conversation store authoritative.
   Prefer an append-only event/transcript log as source of truth.
   Derived JSON snapshots can exist, but they must be generated from the authoritative event stream, not independently mutated.

2. Remove duplicate session persistence paths.
   Do not both log each message immediately and bulk-flush the same turn without dedupe bookkeeping.
   If immediate append exists, bulk flush must track a durable last-flushed offset or become a no-op.

3. Add transcript integrity tests.
   Assert that loading, persisting, and reloading a session does not duplicate turns.
   Assert that JSON snapshot and JSONL transcript agree on the ordered set of user-visible turns.

### P0: Make prompt truth derive from live capability truth

1. Generate platform/tool guidance from the actual visible tool list for the turn.
   If `send_file` is absent, the prompt must not talk about `send_file`.

2. Remove unconditional guidance like:
   "Use discover_capabilities before specialized tools."
   That is backwards. Discovery should be used when the needed tool is missing or unclear.

3. Remove unconditional "use session_search proactively" guidance.
   Session recall should trigger only when:
   - the user explicitly references past work
   - the system detects an unresolved attached run
   - the operator asks for continuity

### P0: Narrow the default channel tool surface

The default conversational surface should be small and obvious:

- reply/respond
- start_work
- check_status
- maybe cancel

Not:

- todo
- memory save
- session_search
- discover_capabilities
- delegate_task
- full artifact plumbing

Those belong behind branch/worker/control-plane logic. This is one of the core places where Spacebot's channel/branch/worker split is the right corrective architecture.

### P0: Turn artifact handling into a typed runtime contract

The model should not have to improvise delivery policy in prose.

Instead:

- worker produces `Artifact { path, kind, suggested_name, visibility }`
- runtime resolves delivery target based on platform and permissions
- CLI defaults to local export
- messaging defaults to attachment delivery
- final user response references a confirmed artifact outcome, not a guessed one

This eliminates a large class of "I think I can send this maybe" confusion.

### P1: Replace diary-memory with typed durable memory

Memory should prefer:

- durable user preferences
- decisions
- verified environment facts
- outcomes
- open commitments

Memory should avoid:

- transient debugging thoughts
- speculative capability claims
- stale implementation notes unless explicitly marked historical

For Spacebot this reinforces the typed graph-memory design:

- save verified memory in structured types
- keep intermediate reasoning disposable
- let branches absorb retrieval noise so channels stay clean

### P1: Move planning overhead out of the user-facing loop

If the user asks for work, the default should be:

- acknowledge briefly
- start the work
- surface progress/status

Not:

- discover capabilities
- create a todo list
- summarize the plan
- maybe ask whether work should proceed

This should happen inside a branch/worker path, not as visible operator tax.

### P1: Add capability confidence and source provenance

Whenever the system believes a capability exists, it should know why:

- live tool exposure
- installed but unavailable dependency
- design-doc only
- memory-derived
- prior session mention only

Then the model can distinguish:

- "I can do this now"
- "I know this was designed, but it is not exposed right now"
- "I remember discussing it, but I need to verify current availability"

Right now those categories are blurred together.

### P1: Separate operator status from model cognition

The runtime knows:

- current run
- available tools
- delivery target
- pending approvals
- active subtask

The model should receive a compact status block derived from those facts, not infer them from prior messages or memory.

This is another place where Spacebot's planned status injection is the right direction.

## Spacebot Implications

The Dross transcripts validate several Spacebot architecture choices:

- channel responsiveness matters
- branch-mediated recall is safer than raw session_search in-channel
- workers should own execution, not the user-facing channel
- memory must be structured and deduplicated
- platform delivery should live in adapters/runtime, not prompt prose
- compaction and recall should be background/system concerns, not visible operator overhead

But they also sharpen some Spacebot requirements:

1. Do not let channels see a giant tool menu.
2. Do not let prompt text drift away from live runtime truth.
3. Do not maintain two conversation stores that can diverge.
4. Do not treat memory as a dumping ground for self-commentary.
5. Do not make the user pay for internal orchestration decisions.

## Recommended Next Moves

### Immediate Dross-side patches

1. Fix duplicate SQLite/session persistence.
2. Remove proactive `discover_capabilities` and `session_search` prompt nudges.
3. Render platform hints from actual tool visibility.
4. Hide most control-plane tools from the default conversational surface.
5. Add transcript-integrity and tool-surface-consistency tests.

### Spacebot-side implementation priorities

1. Keep the channel tool surface tiny.
2. Make branches/workers own planning, recall, and execution.
3. Use one authoritative conversation/event store.
4. Keep memory typed, confidence-aware, and background-curated.
5. Make artifact delivery adapter-owned and typed.

## Bottom Line

Nate's dissatisfaction is justified.

Dross is not mainly suffering from "needs a smarter prompt." It is suffering from a control-plane architecture that keeps making the agent second-guess its own tools, its own memory, and its own conversation state.

If we fix those layers, autonomy rises sharply without needing to beg the model to "be more autonomous."
