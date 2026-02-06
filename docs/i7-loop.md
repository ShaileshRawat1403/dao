# The DAO I7 Loop

> A disciplined thinking and execution loop for AI-assisted software work
> Designed to prevent drift, hallucination, and unsafe automation

**See also:**

- [I7 Loop Integration with Agents and Persona Policy](i7-integration.md)
- [Agent Behavior and Guardrails](agent.md)

---

## I1 — **Initiate**

**What this is**
Decide _why_ something should happen.

**In DAO terms**

- User issues a command, query, or request
- DAO does **nothing** implicitly
- No execution begins without an explicit initiation

**Why it exists**

- Prevents accidental automation
- Forces human agency at the entry point

**DAO surfaces**

- CLI command
- Explicit flags
- Persona / policy selection

---

## I2 — **Interpret**

**What this is**
Understand _what_ is being asked, in context.

**In DAO terms**

- Parse intent
- Load persona policy
- Resolve scope, permissions, and constraints
- Translate natural language into structured intent

**Why it exists**

- Language is ambiguous
- Systems are not
- Interpretation must be visible and inspectable

**DAO surfaces**

- Parsed intent
- Policy tier
- Planned action summary

---

## I3 — **Inspect (Current State)**

**What this is**
Look at reality _before_ changing anything.

**In DAO terms**

- Read filesystem state
- Read git status
- Read configuration
- Read prior context and memory

**Why it exists**

- You cannot reason about a system you haven’t observed
- Most failures come from acting on stale assumptions

**DAO surfaces**

- Read-only probes
- Status panels
- Context snapshots

---

## I4 — **Isolate (Change Scope)**

**What this is**
Constrain _where_ change is allowed.

**In DAO terms**

- Select files
- Select modules
- Define boundaries
- Apply dry-run or no-write modes

**Why it exists**

- Blast radius control
- Prevents “helpful but destructive” behavior

**DAO surfaces**

- File allowlists
- Mode flags (`--dry-run`, `--read-only`)
- Explicit approvals

---

## I5 — **Implement**

**What this is**
Make the smallest correct change.

**In DAO terms**

- Generate code
- Apply edits
- Execute commands (only if allowed)
- Follow workspace conventions

**Why it exists**

- Execution should be boring
- Intelligence is front-loaded, not magical at runtime

**DAO surfaces**

- Deterministic actions
- Logged execution
- Reversible steps

---

## I6 — **Inspect (Results)**

**What this is**
Verify _what actually happened_.

**In DAO terms**

- Run tests
- Check diffs
- Validate outputs
- Compare expected vs actual state

**Why it exists**

- Systems lie less than language
- Verification closes the loop

**DAO surfaces**

- Test results
- Diff views
- Execution logs

---

## I7 — **Integrate or Abort**

**What this is**
Decide whether the change becomes part of the system.

**In DAO terms**

- Commit or discard
- Merge or rollback
- Capture learnings
- Update policy or defaults if needed

**Why it exists**

- Not every correct change should ship
- Learning is part of the system, not a byproduct

**DAO surfaces**

- Commit prompts
- Release gates
- Post-action notes

---

## Why This I7 Exists (DAO Philosophy)

- **No hidden autonomy**
- **No silent execution**
- **No skipped inspection**
- **Human remains accountable**
- **AI remains assistive**

DAO does not replace thinking.
DAO **enforces thinking**.

---

## One-Line Summary

> **Initiate → Interpret → Inspect → Isolate → Implement → Inspect → Integrate**
