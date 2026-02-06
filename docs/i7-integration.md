# I7 Loop Integration with Agents and Persona Policy

This document explains **how the DAO I7 loop is enforced through Agent instructions and Persona policies**.

DAO is not an autonomous agent system.
It is a **policy-governed execution framework**.

---

## Core Relationship

- **I7 Loop** defines _how work progresses_
- **Agent.md** defines _what an agent is allowed to do_
- **Persona Policy** defines _how deep, how fast, and how risky execution can be_

They are not separate concepts.
They are three layers of the same control system.

---

## Mapping Overview

| I7 Stage          | Agent.md Responsibility    | Persona Policy Role          |
| ----------------- | -------------------------- | ---------------------------- |
| Initiate          | Refuse implicit starts     | Require explicit user intent |
| Interpret         | Ask clarifying questions   | Control explanation depth    |
| Inspect (State)   | Read-only by default       | Enforce no-write tiers       |
| Isolate           | Constrain scope            | Limit file / command access  |
| Implement         | Execute only when approved | Gate execution permissions   |
| Inspect (Results) | Report objectively         | Require verification detail  |
| Integrate / Abort | Never auto-commit          | Decide autonomy ceiling      |

---

## Stage-by-Stage Enforcement

---

## I1 — Initiate

**Rule**

> No action without explicit user initiation.

**Agent.md**

- Agent must not “helpfully” begin work
- Must confirm task boundaries before proceeding

**Persona Policy**

- Low tiers force confirmation
- Higher tiers still require explicit triggers

**Failure prevented**

- Accidental execution
- Unintended automation

---

## I2 — Interpret

**Rule**

> Intent must be interpreted, not assumed.

**Agent.md**

- Agent summarizes understanding before acting
- Must surface assumptions

**Persona Policy**

- Controls verbosity
- Controls whether agent can infer or must ask

**Failure prevented**

- Hallucinated requirements
- Silent misinterpretation

---

## I3 — Inspect (Current State)

**Rule**

> Observe reality before changing it.

**Agent.md**

- Read-only inspection first
- No writes during inspection

**Persona Policy**

- Enforces read-only modes
- Blocks execution tools if tier is low

**Failure prevented**

- Acting on stale or false context

---

## I4 — Isolate

**Rule**

> Constrain the blast radius.

**Agent.md**

- Must declare which files or systems are in scope
- Must respect allowlists

**Persona Policy**

- Limits number of files
- Disallows cross-module edits in low tiers

**Failure prevented**

- Over-editing
- Cascading side effects

---

## I5 — Implement

**Rule**

> Execute the smallest correct change.

**Agent.md**

- Execution only after explicit approval
- Must follow workspace standards

**Persona Policy**

- Determines if execution is allowed at all
- Controls use of shell, git, or file writes

**Failure prevented**

- Overengineering
- Unsafe automation

---

## I6 — Inspect (Results)

**Rule**

> Verify outcomes, not intentions.

**Agent.md**

- Must show diffs, logs, or test results
- Cannot declare success without evidence

**Persona Policy**

- Enforces inspection depth
- Requires validation artifacts at higher tiers

**Failure prevented**

- False confidence
- “Looks good” failures

---

## I7 — Integrate or Abort

**Rule**

> Nothing ships automatically.

**Agent.md**

- Agent cannot commit, tag, or release autonomously
- Must present options, not decisions

**Persona Policy**

- Controls whether suggestions are advisory or prescriptive
- Never grants unilateral release authority

**Failure prevented**

- Unreviewed releases
- Irreversible mistakes

---

## Design Principle: Separation of Power

DAO intentionally separates:

- **Thinking (I7)**
- **Capability (Agent.md)**
- **Authority (Persona Policy)**

No single layer is sufficient on its own.

---

## Canonical Guarantee

If an agent:

- Skips an I-step
- Acts outside policy
- Executes without approval

👉 **That is a DAO bug, not user error.**

---

## One-Line Truth

> **DAO is not an AI that acts.
> DAO is a system that refuses to act unsafely.**
